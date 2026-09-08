use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use prost::Message as _;
use prost::bytes::Buf as _;
use protocol::v1::{
    ChannelMeasurement, MeasurementReport, Metric, MetricReading, Modulation, SampleStats,
};

use protocol::{PROTOCOL_VERSION, metric_range};

const PERCENTILE: f64 = 0.95;

/// One metric sample awaiting aggregation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricSample {
    pub channel_frequency_hz: u64,
    pub metric: Metric,
    pub value: f64,
}

/// What a channel is called on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelIdentity {
    pub frequency_hz: u64,
    pub modulation: Modulation,
    pub label: String,
}

#[derive(Debug, Default)]
struct ChannelAccumulator {
    modulation: i32,
    label: String,
    metrics: BTreeMap<i32, Vec<f64>>,
}

/// Everything measured since the window opened.
#[derive(Debug)]
pub struct Aggregator {
    window_start: SystemTime,
    channels: BTreeMap<u64, ChannelAccumulator>,
    clamped: u64,
}

impl Aggregator {
    /// Open a window at `window_start`.
    #[must_use]
    pub const fn new(window_start: SystemTime) -> Self {
        Self {
            window_start,
            channels: BTreeMap::new(),
            clamped: 0,
        }
    }

    /// When the current window opened.
    #[must_use]
    pub const fn window_start(&self) -> SystemTime {
        self.window_start
    }

    /// Whether anything has been recorded into the current window.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// How many samples have been clamped since the agent started.
    #[must_use]
    pub const fn clamped(&self) -> u64 {
        self.clamped
    }

    /// Record one dwell's samples for one channel, clamping to the accepted range.
    pub fn record(&mut self, channel: &ChannelIdentity, samples: &[MetricSample]) {
        if samples.is_empty() {
            return;
        }

        let accumulator = self
            .channels
            .entry(channel.frequency_hz)
            .or_insert_with(|| ChannelAccumulator {
                modulation: i32::from(channel.modulation),
                label: channel.label.clone(),
                metrics: BTreeMap::new(),
            });

        for sample in samples {
            if sample.channel_frequency_hz != channel.frequency_hz {
                continue;
            }
            if !sample.value.is_finite() {
                tracing::warn!(
                    metric = ?sample.metric,
                    frequency_hz = channel.frequency_hz,
                    "dropping a non-finite sample"
                );

                continue;
            }

            let value = match metric_range(sample.metric) {
                Some(range) => {
                    let clamped = sample.value.clamp(*range.start(), *range.end());
                    if (clamped - sample.value).abs() > f64::EPSILON {
                        self.clamped = self.clamped.saturating_add(1);
                        tracing::warn!(
                            metric = ?sample.metric,
                            frequency_hz = channel.frequency_hz,
                            measured = sample.value,
                            reported = clamped,
                            "a sample sat outside the range the server accepts and was clamped"
                        );
                    }

                    clamped
                }
                None => continue,
            };

            accumulator
                .metrics
                .entry(i32::from(sample.metric))
                .or_default()
                .push(value);
        }
    }

    /// Close the window and build the report, opening a fresh window at `window_end`.
    #[must_use]
    pub fn finish(&mut self, window_end: SystemTime) -> Option<MeasurementReport> {
        let window_start = self.window_start;
        let channels = std::mem::take(&mut self.channels);
        self.window_start = window_end;

        if window_end <= window_start {
            tracing::warn!("the aggregation window did not advance, discarding it");

            return None;
        }

        let channels: Vec<ChannelMeasurement> = channels
            .into_iter()
            .filter_map(|(frequency_hz, accumulator)| {
                let readings: Vec<MetricReading> = accumulator
                    .metrics
                    .into_iter()
                    .filter_map(|(metric, mut values)| {
                        Some(MetricReading {
                            metric,
                            stats: Some(summarize(&mut values)?),
                        })
                    })
                    .collect();

                if readings.is_empty() {
                    return None;
                }

                Some(ChannelMeasurement {
                    frequency_hz,
                    modulation: accumulator.modulation,
                    label: accumulator.label,
                    readings,
                })
            })
            .collect();

        if channels.is_empty() {
            return None;
        }

        Some(MeasurementReport {
            protocol_version: PROTOCOL_VERSION.to_owned(),
            window_start: Some(window_start.into()),
            window_end: Some(window_end.into()),
            channels,
        })
    }
}

/// Summarize one metric's samples over a window.
#[must_use]
pub fn summarize(values: &mut [f64]) -> Option<SampleStats> {
    if values.is_empty() {
        return None;
    }

    let count = values.len();
    let widened = crate::dsp::convert::index_to_f64(count);

    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut sum = 0.0_f64;
    for value in values.iter() {
        min = min.min(*value);
        max = max.max(*value);
        sum += *value;
    }
    let mean = sum / widened;

    let variance = if count > 1 {
        let spread: f64 = values.iter().map(|value| (value - mean).powi(2)).sum();

        spread / crate::dsp::convert::index_to_f64(count.saturating_sub(1))
    } else {
        0.0
    };

    values.sort_unstable_by(f64::total_cmp);

    Some(SampleStats {
        min,
        max,
        mean: mean.clamp(min, max),
        median: median(values).unwrap_or(mean).clamp(min, max),
        stddev: variance.sqrt(),
        p95: percentile(values, PERCENTILE)
            .unwrap_or(max)
            .clamp(min, max),
        sample_count: count.try_into().unwrap_or(u64::MAX),
    })
}

fn median(sorted: &[f64]) -> Option<f64> {
    let count = sorted.len();
    if count == 0 {
        return None;
    }

    let upper = sorted.get(count / 2).copied()?;
    if count % 2 == 1 {
        return Some(upper);
    }

    let lower = sorted.get((count / 2).saturating_sub(1)).copied()?;

    Some(f64::midpoint(lower, upper))
}

fn percentile(sorted: &[f64], fraction: f64) -> Option<f64> {
    let count = sorted.len();
    if count == 0 {
        return None;
    }

    let rank = crate::dsp::convert::to_index(
        (fraction * crate::dsp::convert::index_to_f64(count)).ceil(),
        count,
    );

    sorted
        .get(rank.saturating_sub(1).min(count.saturating_sub(1)))
        .copied()
}

/// Reports held on disk until the server takes them.
#[derive(Debug, Clone)]
pub struct ReportBuffer {
    path: PathBuf,
    limit_bytes: u64,
}

impl ReportBuffer {
    /// A buffer backed by `path`, holding at most `limit_bytes`.
    #[must_use]
    pub const fn new(path: PathBuf, limit_bytes: u64) -> Self {
        Self { path, limit_bytes }
    }

    /// Where the journal lives.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Bytes currently held. Zero when the journal does not exist.
    #[must_use]
    pub fn size(&self) -> u64 {
        fs::metadata(&self.path).map_or(0, |meta| meta.len())
    }

    /// Queue a report for delivery.
    ///
    /// # Errors
    ///
    /// [`io::Error`] if the state directory or the journal cannot be written.
    pub fn push(&self, report: &MeasurementReport) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut encoded = Vec::with_capacity(report.encoded_len().saturating_add(8));
        report
            .encode_length_delimited(&mut encoded)
            .map_err(|err| io::Error::other(err.to_string()))?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(&encoded)?;
        file.sync_data()?;

        self.enforce_limit()
    }

    /// Everything pending, oldest first.
    ///
    /// # Errors
    ///
    /// [`io::Error`] if the journal exists but cannot be read.
    pub fn pending(&self) -> io::Result<Vec<MeasurementReport>> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };

        let mut raw = Vec::new();
        file.read_to_end(&mut raw)?;

        Ok(Self::decode_all(&raw))
    }

    /// Replace the journal's contents with the reports still undelivered.
    ///
    /// # Errors
    ///
    /// [`io::Error`] if the replacement cannot be written or renamed.
    pub fn replace(&self, remaining: &[MeasurementReport]) -> io::Result<()> {
        if remaining.is_empty() {
            return match fs::remove_file(&self.path) {
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            };
        }

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut encoded = Vec::new();
        for report in remaining {
            report
                .encode_length_delimited(&mut encoded)
                .map_err(|err| io::Error::other(err.to_string()))?;
        }

        let temporary = self.path.with_extension("pb.tmp");
        let mut file = File::create(&temporary)?;
        file.write_all(&encoded)?;
        file.sync_data()?;
        fs::rename(&temporary, &self.path)
    }

    fn enforce_limit(&self) -> io::Result<()> {
        if self.limit_bytes == 0 || self.size() <= self.limit_bytes {
            return Ok(());
        }

        let mut pending = self.pending()?;
        let before = pending.len();

        let keep = before / 2;
        pending.drain(..before.saturating_sub(keep));

        tracing::warn!(
            dropped = before.saturating_sub(pending.len()),
            limit_bytes = self.limit_bytes,
            "the offline buffer is full, dropping the oldest reports"
        );

        self.replace(&pending)
    }

    fn decode_all(mut raw: &[u8]) -> Vec<MeasurementReport> {
        let mut reports = Vec::new();

        while raw.has_remaining() {
            match MeasurementReport::decode_length_delimited(&mut raw) {
                Ok(report) => reports.push(report),
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        recovered = reports.len(),
                        "the offline buffer ends in a partial record, discarding the tail"
                    );

                    break;
                }
            }
        }

        reports
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn window_end() -> SystemTime {
        SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_mins(1))
            .unwrap_or(SystemTime::UNIX_EPOCH)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "spektra-report-{name}-{}",
            crate::identity::generate()
        ));
        fs::create_dir_all(&dir).expect("the temporary directory is creatable");

        dir
    }

    fn identity() -> ChannelIdentity {
        ChannelIdentity {
            frequency_hz: 89_700_000,
            modulation: Modulation::Fm,
            label: "DR P4 Nordjylland".to_owned(),
        }
    }

    fn sample(metric: Metric, value: f64) -> MetricSample {
        MetricSample {
            channel_frequency_hz: 89_700_000,
            metric,
            value,
        }
    }

    fn report() -> MeasurementReport {
        let mut aggregator = Aggregator::new(SystemTime::UNIX_EPOCH);
        aggregator.record(&identity(), &[sample(Metric::SignalToNoise, 30.0)]);

        aggregator
            .finish(window_end())
            .expect("one recorded sample is a report")
    }

    #[test]
    fn an_empty_window_is_not_a_report() {
        let mut aggregator = Aggregator::new(SystemTime::UNIX_EPOCH);

        assert!(aggregator.is_empty());
        assert!(aggregator.finish(window_end()).is_none());
    }

    #[test]
    fn summarizing_one_sample_reports_no_spread() {
        let stats = summarize(&mut [42.0]).expect("one sample is a summary");

        for (name, value) in [
            ("min", stats.min),
            ("max", stats.max),
            ("mean", stats.mean),
            ("median", stats.median),
            ("p95", stats.p95),
        ] {
            assert!(
                (value - 42.0).abs() < f64::EPSILON,
                "{name} is {value}, not 42"
            );
        }
        assert!(stats.stddev.abs() < f64::EPSILON);
        assert_eq!(stats.sample_count, 1);
    }

    #[test]
    fn summarizing_nothing_is_not_a_summary() {
        assert!(summarize(&mut []).is_none());
    }

    #[test]
    fn the_summary_matches_the_definitions() {
        let mut values = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let stats = summarize(&mut values).expect("eight samples are a summary");

        assert!((stats.min - 2.0).abs() < f64::EPSILON);
        assert!((stats.max - 9.0).abs() < f64::EPSILON);
        assert!((stats.mean - 5.0).abs() < f64::EPSILON);
        assert!((stats.median - 4.5).abs() < f64::EPSILON);
        assert!((stats.stddev - (32.0_f64 / 7.0).sqrt()).abs() < 1e-12);
        assert!((stats.p95 - 9.0).abs() < f64::EPSILON);
        assert_eq!(stats.sample_count, 8);
    }

    #[test]
    fn the_median_averages_the_middle_pair_on_an_even_count() {
        assert!(median(&[1.0, 2.0, 3.0, 4.0]).is_some_and(|v| (v - 2.5).abs() < f64::EPSILON));
        assert!(median(&[1.0, 2.0, 3.0]).is_some_and(|v| (v - 2.0).abs() < f64::EPSILON));
        assert_eq!(median(&[]), None);
    }

    #[test]
    fn every_statistic_lies_inside_the_extremes() {
        let mut values: Vec<f64> = (0..97)
            .map(|index| f64::from(index).mul_add(0.37, -12.0))
            .collect();
        let stats = summarize(&mut values).expect("a summary");

        for (name, value) in [
            ("mean", stats.mean),
            ("median", stats.median),
            ("p95", stats.p95),
        ] {
            assert!(
                value >= stats.min && value <= stats.max,
                "{name} {value} is outside {} to {}",
                stats.min,
                stats.max
            );
        }
        assert!(stats.stddev >= 0.0);
    }

    #[test]
    fn a_sample_outside_the_accepted_range_is_clamped_not_dropped() {
        let mut aggregator = Aggregator::new(SystemTime::UNIX_EPOCH);
        aggregator.record(
            &identity(),
            &[
                sample(Metric::SignalToNoise, 95.0),
                sample(Metric::SignalToNoise, -3.0),
                sample(Metric::SignalStrength, 12.0),
            ],
        );

        let report = aggregator.finish(window_end()).expect("a report");
        let channel = report.channels.first().expect("one channel");

        for reading in &channel.readings {
            let stats = reading.stats.as_ref().expect("stats are present");
            let range = metric_range(Metric::try_from(reading.metric).unwrap_or_default())
                .expect("a known metric");

            assert!(stats.min >= *range.start() && stats.max <= *range.end());
        }
        assert_eq!(aggregator.clamped(), 3);
    }

    #[test]
    fn a_non_finite_sample_is_dropped() {
        let mut aggregator = Aggregator::new(SystemTime::UNIX_EPOCH);
        aggregator.record(
            &identity(),
            &[
                sample(Metric::SignalToNoise, f64::NAN),
                sample(Metric::SignalToNoise, f64::INFINITY),
            ],
        );

        assert!(
            aggregator.finish(window_end()).is_none(),
            "nothing finite was measured, so there is nothing to report"
        );
    }

    #[test]
    fn a_report_carries_the_window_and_the_channel() {
        let report = report();

        assert_eq!(report.protocol_version, PROTOCOL_VERSION);
        assert_eq!(report.channels.len(), 1);

        let channel = report.channels.first().expect("one channel");
        assert_eq!(channel.frequency_hz, 89_700_000);
        assert_eq!(channel.modulation, i32::from(Modulation::Fm));
        assert_eq!(channel.label, "DR P4 Nordjylland");
    }

    #[test]
    fn a_window_that_does_not_advance_is_discarded() {
        let mut aggregator = Aggregator::new(SystemTime::UNIX_EPOCH);
        aggregator.record(&identity(), &[sample(Metric::SignalToNoise, 30.0)]);

        assert!(aggregator.finish(SystemTime::UNIX_EPOCH).is_none());
    }

    #[test]
    fn a_buffer_round_trips_its_reports() {
        let dir = scratch("roundtrip");
        let buffer = ReportBuffer::new(dir.join("pending.pb"), 0);

        assert!(
            buffer
                .pending()
                .expect("an absent journal is empty")
                .is_empty()
        );

        for _ in 0..3 {
            buffer.push(&report()).expect("the journal is writable");
        }

        let pending = buffer.pending().expect("the journal is readable");
        assert_eq!(pending.len(), 3);
        assert_eq!(pending.first().map(|r| r.channels.len()), Some(1));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn replacing_with_nothing_removes_the_journal() {
        let dir = scratch("drain");
        let buffer = ReportBuffer::new(dir.join("pending.pb"), 0);
        buffer.push(&report()).expect("writable");

        buffer.replace(&[]).expect("the journal is removable");

        assert_eq!(buffer.size(), 0);
        assert!(buffer.pending().expect("readable").is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_partial_trailing_record_does_not_lose_the_whole_journal() {
        let dir = scratch("torn");
        let path = dir.join("pending.pb");
        let buffer = ReportBuffer::new(path.clone(), 0);
        buffer.push(&report()).expect("writable");
        buffer.push(&report()).expect("writable");

        let mut raw = fs::read(&path).expect("readable");
        raw.extend_from_slice(&[0x80, 0x02, 0x0A]);
        fs::write(&path, raw).expect("writable");

        assert_eq!(buffer.pending().expect("readable").len(), 2);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_full_buffer_drops_its_oldest_reports() {
        let dir = scratch("full");
        let buffer = ReportBuffer::new(dir.join("pending.pb"), 200);

        for _ in 0..40 {
            buffer.push(&report()).expect("writable");
        }

        assert!(
            buffer.size() <= 400,
            "the journal grew to {}",
            buffer.size()
        );
        let pending = buffer.pending().expect("readable");
        assert!(!pending.is_empty(), "the trim took everything");

        fs::remove_dir_all(&dir).ok();
    }
}
