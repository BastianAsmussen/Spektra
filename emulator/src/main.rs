mod signal;

use std::time::{Duration, SystemTime};

use clap::Parser;
use color_eyre::Result;
use color_eyre::eyre::WrapErr;
use protocol::schedule_delay;
use protocol::v1::node_ingest_client::NodeIngestClient;
use protocol::v1::{
    Capabilities, ChannelMeasurement, Hardware, HealthReport, Location, MeasurementReport, Metric,
    MetricReading, Modulation, NodeRegistrationRequest, SampleStats,
};
use signal::{Sample, Signal};
use tokio::task::JoinSet;
use tonic::Request;
use tonic::metadata::MetadataMap;
use tonic::transport::{Channel, ClientTlsConfig};

const PLAN: [(u64, Modulation, &str); 6] = [
    (89_700_000, Modulation::Fm, "DR P4 Nordjylland"),
    (93_900_000, Modulation::Fm, "DR P3"),
    (96_500_000, Modulation::Fm, "DR P1"),
    (102_300_000, Modulation::Fm, "Radio4"),
    (227_360_000, Modulation::Dab, "DAB blok 12B"),
    (223_936_000, Modulation::Dab, "DAB blok 11D"),
];

/// Register a fake node, backfill its history, then keep reporting.
#[derive(Parser, Debug)]
#[command(author, version, about, allow_negative_numbers = true)]
struct Args {
    /// gRPC endpoint of the server.
    #[arg(short, long, default_value = "http://localhost:50051")]
    server: String,

    /// Stable node identity, e.g. a UUID. Must be unique across the fleet.
    #[arg(short, long)]
    identity: String,

    /// Enrollment token minted for this node, which registration now requires.
    #[arg(short = 't', long)]
    enrollment_token: String,

    /// Human-readable node name.
    #[arg(short, long, default_value = "emulator")]
    name: String,

    /// Latitude of the fake antenna.
    #[arg(long, default_value_t = 57.05)]
    latitude: f64,

    /// Longitude of the fake antenna.
    #[arg(long, default_value_t = 9.92)]
    longitude: f64,

    /// Hours of history to submit before going live.
    #[arg(long, default_value_t = 168)]
    backfill_hours: i64,

    /// Length of one aggregation window, in seconds.
    #[arg(long, default_value_t = 60)]
    window_seconds: i64,

    /// How many of the channels in the plan to watch, 1 to 6.
    #[arg(long, default_value_t = 4)]
    channels: usize,

    /// Backfill reports in flight at once.
    #[arg(long, default_value_t = 16)]
    concurrency: usize,

    /// Seconds between live reports once the backfill is in.
    #[arg(long, default_value_t = 60)]
    interval_seconds: u64,

    /// Stop after the backfill instead of staying live.
    #[arg(long)]
    once: bool,
}

fn bearer(token: &str) -> Result<MetadataMap> {
    let mut metadata = MetadataMap::new();
    metadata.insert(
        "authorization",
        format!("Bearer {token}")
            .parse()
            .wrap_err("credential is not a valid metadata value")?,
    );
    Ok(metadata)
}

const fn spread(mean: f64, deviation: f64, samples: u64) -> SampleStats {
    SampleStats {
        min: deviation.mul_add(-2.1, mean),
        max: deviation.mul_add(1.9, mean),
        mean,
        median: deviation.mul_add(-0.05, mean),
        stddev: deviation,
        p95: deviation.mul_add(1.6, mean),
        sample_count: samples,
    }
}

fn readings(sample: Sample, index: usize) -> Vec<MetricReading> {
    let bias = index_bias(index);

    vec![
        MetricReading {
            metric: i32::from(Metric::SignalStrength),
            stats: Some(spread(bias.mul_add(-1.4, sample.strength_db), 0.62, 60)),
        },
        MetricReading {
            metric: i32::from(Metric::SignalToNoise),
            stats: Some(spread(bias.mul_add(-0.9, sample.snr_db), 0.51, 60)),
        },
        MetricReading {
            metric: i32::from(Metric::CarrierOffset),
            stats: Some(spread(bias.mul_add(6.0, sample.offset_hz), 13.4, 60)),
        },
        MetricReading {
            metric: i32::from(Metric::DemodErrorRate),
            stats: Some(spread(sample.error_rate.max(0.0002), 0.0004, 60)),
        },
        MetricReading {
            metric: i32::from(Metric::SpectrumOccupancy),
            stats: Some(spread(sample.occupancy, 0.018, 60)),
        },
    ]
}

fn index_bias(index: usize) -> f64 {
    f64::from(u32::try_from(index).unwrap_or(0)) * 0.5
}

fn report(
    signal: Signal,
    channels: usize,
    age_seconds: i64,
    window_seconds: i64,
) -> MeasurementReport {
    let now = SystemTime::now();
    let age = Duration::from_secs(age_seconds.max(0).unsigned_abs());
    let window = Duration::from_secs(window_seconds.max(1).unsigned_abs());

    let window_end = now.checked_sub(age).unwrap_or(now);
    let window_start = window_end.checked_sub(window).unwrap_or(window_end);

    let sample = signal.at(seconds(age_seconds));

    MeasurementReport {
        protocol_version: protocol::PROTOCOL_VERSION.to_owned(),
        window_start: Some(window_start.into()),
        window_end: Some(window_end.into()),
        channels: PLAN
            .iter()
            .take(channels.clamp(1, PLAN.len()))
            .enumerate()
            .map(
                |(index, (frequency_hz, modulation, label))| ChannelMeasurement {
                    frequency_hz: *frequency_hz,
                    modulation: i32::from(*modulation),
                    label: (*label).to_owned(),
                    readings: readings(sample, index),
                },
            )
            .collect(),
    }
}

fn seconds(value: i64) -> f64 {
    f64::from(i32::try_from(value.clamp(0, i64::from(i32::MAX))).unwrap_or(0))
}

fn health(signal: Signal, age_seconds: i64) -> HealthReport {
    let sample = signal.at(seconds(age_seconds));
    let strain = (28.4 - sample.snr_db).max(0.0);

    HealthReport {
        protocol_version: protocol::PROTOCOL_VERSION.to_owned(),
        measured_at: Some(
            SystemTime::now()
                .checked_sub(Duration::from_secs(age_seconds.max(0).unsigned_abs()))
                .unwrap_or_else(SystemTime::now)
                .into(),
        ),
        uptime_seconds: 86_400.0 + seconds(age_seconds),
        load_1m: strain.mul_add(0.12, 0.35),
        load_5m: strain.mul_add(0.09, 0.31),
        load_15m: strain.mul_add(0.05, 0.24),
        cpu_temperature_celsius: strain.mul_add(3.1, 44.5),
        clock_offset_seconds: strain.mul_add(0.02, 0.05),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::parse();

    let channel = Channel::from_shared(args.server.clone())
        .wrap_err_with(|| format!("invalid server address '{}'", args.server))?
        .tls_config(ClientTlsConfig::new().with_enabled_roots())
        .wrap_err("could not configure TLS")?
        .connect()
        .await
        .wrap_err("failed to connect")?;

    let mut client = NodeIngestClient::new(channel);
    let signal = Signal::for_identity(&args.identity);

    let registration = NodeRegistrationRequest {
        protocol_version: protocol::PROTOCOL_VERSION.to_owned(),
        identity: args.identity.clone(),
        name: args.name.clone(),
        location: Some(Location {
            latitude: args.latitude,
            longitude: args.longitude,
        }),
        hardware: Some(Hardware {
            device: "RTL-SDR emulator".to_owned(),
            antenna: "emulated dipole".to_owned(),
            max_sample_rate_hz: 2_400_000,
        }),
        capabilities: Some(Capabilities {
            metrics: vec![
                i32::from(Metric::SignalStrength),
                i32::from(Metric::SignalToNoise),
                i32::from(Metric::CarrierOffset),
                i32::from(Metric::DemodErrorRate),
                i32::from(Metric::SpectrumOccupancy),
            ],
            modulations: vec![i32::from(Modulation::Fm), i32::from(Modulation::Dab)],
        }),
    };

    let mut request = Request::new(registration);
    *request.metadata_mut() = bearer(&args.enrollment_token)?;
    let response = client
        .register_node(request)
        .await
        .wrap_err_with(|| format!("registration of {} failed", args.identity))?
        .into_inner();

    println!(
        "{} ({}): registered as node {}{}",
        args.identity,
        args.name,
        response.node_id,
        if signal.is_faulty() {
            ", degrading"
        } else {
            ""
        }
    );

    let scheduled = schedule_delay(response.schedule.as_ref(), response.server_time.as_ref());
    let credential = response.credential;
    let backfilled = backfill(&client, &credential, signal, &args).await?;
    println!(
        "{}: backfilled {backfilled} windows over {} channels",
        args.identity,
        args.channels.clamp(1, PLAN.len())
    );

    if args.once {
        return Ok(());
    }

    live(&mut client, &credential, signal, &args, scheduled).await
}

async fn backfill(
    client: &NodeIngestClient<Channel>,
    credential: &str,
    signal: Signal,
    args: &Args,
) -> Result<u64> {
    let window = args.window_seconds.max(1);
    let span = args.backfill_hours.max(0).saturating_mul(3_600);
    let Some(total) = span.checked_div(window).filter(|total| *total > 0) else {
        return Ok(0);
    };

    let mut sent = 0_u64;
    let mut failed = 0_u64;
    let mut inflight = JoinSet::new();
    let limit = args.concurrency.max(1);

    for step in (1..=total).rev() {
        let age = step.saturating_mul(window);
        let mut request = Request::new(report(signal, args.channels, age, window));
        *request.metadata_mut() = bearer(credential)?;

        let mut client = client.clone();
        inflight.spawn(async move { client.submit_measurements(request).await.is_ok() });

        if inflight.len() >= limit
            && let Some(result) = inflight.join_next().await
        {
            tally(result.unwrap_or(false), &mut sent, &mut failed);
        }
    }

    while let Some(result) = inflight.join_next().await {
        tally(result.unwrap_or(false), &mut sent, &mut failed);
    }

    let samples = total.clamp(1, 48);
    let stride = span.checked_div(samples).unwrap_or(window);
    for step in (0..=samples).rev() {
        let age = step.saturating_mul(stride);
        let mut request = Request::new(health(signal, age));
        *request.metadata_mut() = bearer(credential)?;
        drop(client.clone().report_health(request).await);
    }

    if failed > 0 {
        eprintln!("{}: {failed} windows rejected", args.identity);
    }

    Ok(sent)
}

const fn tally(accepted: bool, sent: &mut u64, failed: &mut u64) {
    if accepted {
        *sent = sent.saturating_add(1);
    } else {
        *failed = failed.saturating_add(1);
    }
}

async fn live(
    client: &mut NodeIngestClient<Channel>,
    credential: &str,
    signal: Signal,
    args: &Args,
    scheduled: Option<Duration>,
) -> Result<()> {
    let fallback = Duration::from_secs(args.interval_seconds.max(1));
    let mut wait = scheduled.unwrap_or(fallback);
    println!(
        "{}: live, one window every {}s (ctrl-c to stop)",
        args.identity, args.interval_seconds
    );

    loop {
        tokio::time::sleep(wait).await;

        let mut request = Request::new(report(signal, args.channels, 0, args.window_seconds));
        *request.metadata_mut() = bearer(credential)?;
        match client.submit_measurements(request).await {
            Ok(response) => {
                let ack = response.into_inner();
                wait = schedule_delay(ack.schedule.as_ref(), ack.server_time.as_ref())
                    .unwrap_or(fallback);
            }
            Err(status) => eprintln!("{}: measurement rejected: {status}", args.identity),
        }

        let mut request = Request::new(health(signal, 0));
        *request.metadata_mut() = bearer(credential)?;
        if let Err(status) = client.report_health(request).await {
            eprintln!("{}: health rejected: {status}", args.identity);
        }
    }
}
