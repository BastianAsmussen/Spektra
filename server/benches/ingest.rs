use std::hint::black_box;
use std::time::{Duration, SystemTime};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use protocol::v1::{
    ChannelMeasurement, MeasurementReport, Metric, MetricReading, Modulation, SampleStats,
};
use server::grpc::persist::PreparedReport;
use server::grpc::validate;

const BASE_FREQUENCY_HZ: u64 = 87_700_000;

/// 200 kHz so 32 synthetic channels stay inside the 87.5 to 108 MHz band validation enforces.
const CHANNEL_SPACING_HZ: u64 = 200_000;

const SAMPLE_COUNT: u64 = 60;

const METRICS: [(Metric, f64, f64); 5] = [
    (Metric::SignalStrength, -48.2, -44.9),
    (Metric::SignalToNoise, 27.1, 29.8),
    (Metric::CarrierOffset, 52.0, 121.0),
    (Metric::DemodErrorRate, 0.0, 0.02),
    (Metric::SpectrumOccupancy, 0.78, 0.86),
];

fn reading(metric: Metric, min: f64, max: f64) -> MetricReading {
    let middle = f64::midpoint(min, max);

    MetricReading {
        metric: i32::from(metric),
        stats: Some(SampleStats {
            min,
            max,
            mean: middle,
            median: middle,
            stddev: 0.5,
            p95: max,
            sample_count: SAMPLE_COUNT,
        }),
    }
}

fn channel(index: u64) -> ChannelMeasurement {
    let offset = index.saturating_mul(CHANNEL_SPACING_HZ);

    ChannelMeasurement {
        frequency_hz: BASE_FREQUENCY_HZ.saturating_add(offset),
        modulation: i32::from(Modulation::Fm),
        label: format!("synthetic channel {index}"),
        readings: METRICS
            .into_iter()
            .map(|(metric, min, max)| reading(metric, min, max))
            .collect(),
    }
}

fn report(channels: u64) -> MeasurementReport {
    let now = SystemTime::now();
    let window_start = now.checked_sub(Duration::from_mins(1)).unwrap_or(now);

    MeasurementReport {
        protocol_version: "1".to_owned(),
        window_start: Some(window_start.into()),
        window_end: Some(now.into()),
        channels: (0..channels).map(channel).collect(),
    }
}

fn ingest(c: &mut Criterion) {
    let mut group = c.benchmark_group("ingest");

    for channels in [1_u64, 8, 32] {
        let report = report(channels);

        let readings: usize = report
            .channels
            .iter()
            .map(|channel| channel.readings.len())
            .sum();
        group.throughput(Throughput::Elements(
            u64::try_from(readings).unwrap_or(u64::MAX),
        ));

        group.bench_with_input(
            BenchmarkId::new("validate", channels),
            &report,
            |bencher, report| bencher.iter(|| validate::measurements(black_box(report))),
        );

        group.bench_with_input(
            BenchmarkId::new("prepare", channels),
            &report,
            |bencher, report| {
                bencher.iter_batched(
                    || report.clone(),
                    |owned| PreparedReport::try_from(black_box(owned)),
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(benches, ingest);
criterion_main!(benches);
