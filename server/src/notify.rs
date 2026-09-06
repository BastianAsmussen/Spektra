use std::time::Duration;

use serde::Serialize;

use crate::db::models::enums::Metric;

const URL_ENV: &str = "SPEKTRA_NTFY_URL";
const TOPIC_ENV: &str = "SPEKTRA_NTFY_TOPIC";
const TOKEN_ENV: &str = "SPEKTRA_NTFY_TOKEN";

const TIMEOUT: Duration = Duration::from_secs(5);

/// Severity in band half-widths at which an alarm is pushed at high priority.
const URGENT_SEVERITY: f64 = 3.0;

/// A configured ntfy publisher.
#[derive(Debug, Clone)]
pub struct Ntfy {
    client: reqwest::Client,
    endpoint: String,
    topic: String,
    token: Option<String>,
}

/// What one alarm looks like to a phone.
#[derive(Debug, Clone)]
pub struct Notice {
    pub alarm_id: i64,
    pub node_id: i64,
    pub node_name: String,
    pub metric: Option<Metric>,
    pub severity: Option<f64>,
    pub summary: String,
}

#[derive(Debug, Serialize)]
struct Payload<'a> {
    topic: &'a str,
    title: String,
    message: &'a str,
    tags: Vec<&'static str>,
    priority: u8,
}

impl Ntfy {
    /// Build a publisher from the environment, or `None` when it is not configured.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let base = std::env::var(URL_ENV).ok()?;
        let topic = std::env::var(TOPIC_ENV).ok()?;
        if base.trim().is_empty() || topic.trim().is_empty() {
            return None;
        }

        let client = reqwest::Client::builder().timeout(TIMEOUT).build().ok()?;

        Some(Self {
            client,
            endpoint: format!("{}/", base.trim_end_matches('/')),
            topic: topic.trim().to_owned(),
            token: std::env::var(TOKEN_ENV)
                .ok()
                .filter(|t| !t.trim().is_empty()),
        })
    }

    /// Build a publisher pointing at an explicit endpoint.
    ///
    /// # Errors
    ///
    /// Returns the reqwest error if a client cannot be built.
    pub fn new(base_url: &str, topic: &str, token: Option<String>) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: reqwest::Client::builder().timeout(TIMEOUT).build()?,
            endpoint: format!("{}/", base_url.trim_end_matches('/')),
            topic: topic.to_owned(),
            token,
        })
    }

    /// Push one alarm, logging rather than propagating a failure.
    pub async fn publish(&self, notice: &Notice) {
        let payload = Payload {
            topic: &self.topic,
            title: title(notice),
            message: &notice.summary,
            tags: tags(notice),
            priority: priority(notice),
        };

        let mut request = self.client.post(&self.endpoint).json(&payload);
        if let Some(token) = self.token.as_deref() {
            request = request.bearer_auth(token);
        }

        match request.send().await {
            Ok(response) if response.status().is_success() => {
                tracing::debug!(alarm_id = notice.alarm_id, topic = %self.topic, "alarm pushed");
            }
            Ok(response) => {
                tracing::warn!(
                    alarm_id = notice.alarm_id,
                    status = %response.status(),
                    "ntfy refused the alarm"
                );
            }
            Err(err) => {
                let alarm_id = notice.alarm_id;
                tracing::warn!(error = %err, alarm_id, "could not push the alarm");
            }
        }
    }
}

fn title(notice: &Notice) -> String {
    notice.metric.map_or_else(
        || format!("{} is silent", notice.node_name),
        |metric| format!("{} on {}", metric.label(), notice.node_name),
    )
}

fn tags(notice: &Notice) -> Vec<&'static str> {
    let mut tags = vec![match notice.metric {
        None => "no_entry",
        Some(_) => "warning",
    }];

    if notice
        .severity
        .is_some_and(|severity| severity >= URGENT_SEVERITY)
    {
        tags.push("rotating_light");
    }

    tags
}

fn priority(notice: &Notice) -> u8 {
    match notice.severity {
        None => 4,
        Some(severity) if severity >= URGENT_SEVERITY => 5,
        Some(_) => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(metric: Option<Metric>, severity: Option<f64>) -> Notice {
        Notice {
            alarm_id: 1,
            node_id: 2,
            node_name: "Hadsund".to_owned(),
            metric,
            severity,
            summary: "signal to noise fell to 12.0 dB".to_owned(),
        }
    }

    #[test]
    fn a_metric_alarm_names_the_metric_and_the_node() {
        assert_eq!(
            title(&notice(Some(Metric::SignalToNoise), Some(2.0))),
            "signal_to_noise on Hadsund"
        );
    }

    #[test]
    fn a_silent_node_says_so() {
        assert_eq!(title(&notice(None, None)), "Hadsund is silent");
    }

    #[test]
    fn a_silent_node_wakes_somebody() {
        assert_eq!(priority(&notice(None, None)), 4);
    }

    #[test]
    fn a_marginal_deviation_waits_for_the_morning() {
        assert_eq!(priority(&notice(Some(Metric::SignalToNoise), Some(1.2))), 3);
    }

    #[test]
    fn a_bad_deviation_is_urgent() {
        let bad = notice(Some(Metric::SignalToNoise), Some(6.0));

        assert_eq!(priority(&bad), 5);
        assert!(tags(&bad).contains(&"rotating_light"));
    }

    #[test]
    fn tags_stay_short() {
        for case in [
            notice(None, None),
            notice(Some(Metric::SignalStrength), Some(1.0)),
            notice(Some(Metric::SignalStrength), Some(9.0)),
        ] {
            assert!(tags(&case).len() <= 2, "{:?}", tags(&case));
        }
    }

    #[test]
    fn an_unconfigured_environment_disables_pushing() {
        assert!(
            std::env::var(URL_ENV).is_err() || std::env::var(TOPIC_ENV).is_err(),
            "the test environment must not be configured for ntfy"
        );
        assert!(Ntfy::from_env().is_none());
    }

    #[test]
    fn an_endpoint_never_ends_in_a_double_slash() {
        let with = Ntfy::new("https://ntfy.example.org/", "spektra", None).expect("a client");
        let without = Ntfy::new("https://ntfy.example.org", "spektra", None).expect("a client");

        assert_eq!(with.endpoint, "https://ntfy.example.org/");
        assert_eq!(without.endpoint, "https://ntfy.example.org/");
    }
}
