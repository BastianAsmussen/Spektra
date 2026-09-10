use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use color_eyre::Result;
use color_eyre::eyre::{Context as _, eyre};
use node_agent::client::{Client, ClientError};
use node_agent::config::{Config, Receiver};
use node_agent::health::Health;
use node_agent::identity::Identity;
use node_agent::live::{self, Live};
use node_agent::plan as plan_cache;
use node_agent::report::{Aggregator, ChannelIdentity, ReportBuffer};
use node_agent::sampler::{self, Assignment, Event, Plan, Sampler};
use node_agent::source::{self, Synthetic};
use protocol::v1::{ChannelPlan, Modulation};
use tokio::sync::{mpsc, watch};
use tracing_subscriber::EnvFilter;

const REGISTRATION_RETRY: Duration = Duration::from_secs(30);
const DEFAULT_FILTER: &str = "node_agent=info,warn";

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER)),
        )
        .init();

    let config = Config::from_args().wrap_err("the configuration is not usable")?;
    tracing::info!(
        server = %config.server,
        state_dir = %config.state_dir.display(),
        receiver = ?config.receiver,
        "starting"
    );

    let mut client = Client::new(&config.server).wrap_err("could not build a client")?;
    let identity = register(&mut client, &config).await?;

    let cached = plan_cache::load(&config.plan_path());
    let (plan_tx, plan_rx) = watch::channel(cached.as_ref().map(translate).unwrap_or_default());
    if let Some(restored) = cached.as_ref() {
        tracing::info!(
            version = restored.plan_version,
            channels = restored.channels.len(),
            "restored the cached channel plan"
        );
    }
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let running = Arc::new(AtomicBool::new(true));
    let sampler = start_sampler(&config, plan_rx, events_tx, &running)?;

    let buffer = ReportBuffer::new(config.pending_path(), config.buffer_limit_bytes);
    let mut aggregator = Aggregator::new(SystemTime::now());

    let (live, watcher) = live::spawn(client.duplicate());
    let watcher = tokio::spawn(watcher);

    run(
        &config,
        &mut client,
        &buffer,
        &mut aggregator,
        events_rx,
        &plan_tx,
        &live,
    )
    .await;

    watcher.abort();

    shutdown(&running, sampler, &mut client, &buffer, &mut aggregator).await;
    tracing::info!(node_id = identity.node_id, "stopped");

    Ok(())
}

async fn register(client: &mut Client, config: &Config) -> Result<Identity> {
    loop {
        match client.register_or_load(config).await {
            Ok(identity) => return Ok(identity),
            Err(ClientError::LostCredential { identity }) => {
                return Err(eyre!(
                    "the server still knows identity '{identity}' but this node no longer holds its credential; re-issue one server-side rather than re-registering"
                ));
            }
            Err(err) => {
                tracing::warn!(error = %err, "registration failed, retrying");
                tokio::time::sleep(REGISTRATION_RETRY).await;
            }
        }
    }
}

fn start_sampler(
    config: &Config,
    plan: watch::Receiver<Plan>,
    events: mpsc::UnboundedSender<Event>,
    running: &Arc<AtomicBool>,
) -> Result<JoinHandle<()>> {
    let running = Arc::clone(running);

    match config.receiver {
        Receiver::Synthetic => {
            tracing::warn!(
                "running on a synthetic carrier; no receiver is being read and these numbers are not measurements"
            );
            let sampler =
                Sampler::new(Synthetic::new(config.device), config.fft_size, config.dwell)
                    .wrap_err("the transform length is not a power of two")?;

            Ok(sampler::spawn(sampler, plan, events, running))
        }
        Receiver::RtlSdr | Receiver::AirspyMini => {
            let source = source::open(&config.device)
                .map_err(|err| eyre!("could not open the receiver: {err}"))?;
            tracing::info!(device = %config.device.device, "receiver open");

            let sampler = Sampler::new(source, config.fft_size, config.dwell)
                .wrap_err("the transform length is not a power of two")?;

            Ok(sampler::spawn(sampler, plan, events, running))
        }
    }
}

async fn run(
    config: &Config,
    client: &mut Client,
    buffer: &ReportBuffer,
    aggregator: &mut Aggregator,
    mut events: mpsc::UnboundedReceiver<Event>,
    plan: &watch::Sender<Plan>,
    live: &Live,
) {
    let mut window = tokio::time::interval(config.window);
    let mut health = tokio::time::interval(config.health_interval);
    let mut poll = tokio::time::interval(config.plan_interval);
    window.tick().await;
    health.tick().await;

    let delivery = tokio::time::sleep(client.delivery_delay().unwrap_or(config.window));
    tokio::pin!(delivery);

    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(Event::Measured { channel, samples }) => {
                    live.offer(&channel, &samples);
                    aggregator.record(&channel, &samples);
                }
                Some(Event::Failed { frequency_hz, reason }) => {
                    tracing::warn!(frequency_hz, reason, "a dwell produced nothing");
                }
                None => {
                    tracing::error!("the sampler stopped, shutting down");

                    return;
                }
            },
            _ = window.tick() => close_window(aggregator, buffer),
            () = delivery.as_mut() => {
                deliver(client, buffer).await;

                let next = client.delivery_delay().unwrap_or(config.window);
                let at = tokio::time::Instant::now()
                    .checked_add(next)
                    .unwrap_or_else(tokio::time::Instant::now);
                delivery.as_mut().reset(at);
            }
            _ = health.tick() => {
                if let Err(err) = client.report_health(Health::read()).await {
                    tracing::warn!(error = %err, "the health report was not delivered");
                }
            }
            _ = poll.tick() => {
                if !refresh_plan(client, plan, &config.plan_path()).await {
                    return;
                }
            }
            result = tokio::signal::ctrl_c() => {
                match result {
                    Ok(()) => tracing::info!("shutting down"),
                    Err(err) => tracing::error!(error = %err, "could not install a signal handler"),
                }

                return;
            }
        }
    }
}

async fn shutdown(
    running: &Arc<AtomicBool>,
    sampler: JoinHandle<()>,
    client: &mut Client,
    buffer: &ReportBuffer,
    aggregator: &mut Aggregator,
) {
    running.store(false, Ordering::Relaxed);
    if sampler.join().is_err() {
        tracing::error!("the sampler thread did not shut down cleanly");
    }

    close_window(aggregator, buffer);
    deliver(client, buffer).await;
}

fn close_window(aggregator: &mut Aggregator, buffer: &ReportBuffer) {
    let Some(report) = aggregator.finish(SystemTime::now()) else {
        return;
    };

    if let Err(err) = buffer.push(&report) {
        tracing::error!(error = %err, "could not buffer a report; it is lost");
    }
}

async fn refresh_plan(
    client: &mut Client,
    plan: &watch::Sender<Plan>,
    cache: &std::path::Path,
) -> bool {
    let known = plan.borrow().version;

    let served = match client.channel_plan(known).await {
        Ok(served) => served,
        Err(err) => {
            tracing::warn!(error = %err, "could not fetch the channel plan");

            return true;
        }
    };

    if served.plan_version == known {
        return true;
    }

    if let Err(err) = plan_cache::store(cache, &served) {
        tracing::warn!(error = %err, "could not cache the channel plan");
    }

    let translated = translate(&served);
    tracing::info!(
        version = translated.version,
        channels = translated.channels.len(),
        "channel plan updated"
    );

    plan.send(translated).is_ok()
}

async fn deliver(client: &mut Client, buffer: &ReportBuffer) {
    let pending = match buffer.pending() {
        Ok(pending) => pending,
        Err(err) => {
            tracing::error!(error = %err, "could not read the offline buffer");

            return;
        }
    };
    if pending.is_empty() {
        return;
    }

    let mut delivered: usize = 0;
    for report in &pending {
        match client.submit(report.clone()).await {
            Ok(ack) => {
                delivered = delivered.saturating_add(1);
                tracing::debug!(
                    accepted_channels = ack.accepted_channels,
                    "a report was accepted"
                );
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    queued = pending.len().saturating_sub(delivered),
                    "delivery failed, the rest stays buffered"
                );

                break;
            }
        }
    }

    if delivered == 0 {
        return;
    }

    let remaining: Vec<_> = pending.into_iter().skip(delivered).collect();
    if let Err(err) = buffer.replace(&remaining) {
        tracing::error!(error = %err, "could not trim the offline buffer");
    }

    tracing::info!(delivered, "reports delivered");
}

fn translate(served: &ChannelPlan) -> Plan {
    Plan {
        version: served.plan_version,
        channels: served
            .channels
            .iter()
            .map(|channel| {
                let modulation = Modulation::try_from(channel.modulation).unwrap_or_default();
                let label = if channel.label.is_empty() {
                    format!("{} Hz", channel.frequency_hz)
                } else {
                    channel.label.clone()
                };

                Assignment {
                    identity: ChannelIdentity {
                        frequency_hz: channel.frequency_hz,
                        modulation,
                        label,
                    },
                    bandwidth_hz: channel.bandwidth_hz,
                }
            })
            .collect(),
    }
}
