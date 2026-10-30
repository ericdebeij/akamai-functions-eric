use serde::Deserialize;
use std::collections::HashSet;

pub const MAX_IDS: usize = 5000;
pub const MAX_WEBHOOK_BODY_BYTES: usize = 1_048_576;

#[derive(Debug, PartialEq, Eq)]
pub enum WebhookError {
    InvalidJson,
    MissingStreamIds,
    TooManyIds,
}

#[derive(Deserialize)]
struct Webhook {
    alerts: Vec<Alert>,
}

#[derive(Deserialize)]
struct Alert {
    status: String,
    labels: Option<Labels>,
}

#[derive(Deserialize)]
struct Labels {
    stream_ids: Option<String>,
}

pub fn extract_firing_ids(body: &[u8]) -> Result<Vec<String>, WebhookError> {
    let webhook: Webhook = serde_json::from_slice(body).map_err(|_| WebhookError::InvalidJson)?;

    let mut ids = Vec::new();
    let mut unique_ids = HashSet::new();
    let mut saw_firing_alert = false;

    for alert in webhook
        .alerts
        .into_iter()
        .filter(|alert| alert.status == "firing")
    {
        saw_firing_alert = true;
        let stream_ids = alert
            .labels
            .and_then(|labels| labels.stream_ids)
            .ok_or(WebhookError::MissingStreamIds)?;

        for id in stream_ids
            .split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            if unique_ids.insert(id.to_owned()) {
                if ids.len() == MAX_IDS {
                    return Err(WebhookError::TooManyIds);
                }
                ids.push(id.to_owned());
            }
        }
    }

    if saw_firing_alert && ids.is_empty() {
        return Err(WebhookError::MissingStreamIds);
    }

    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_firing_fixture_and_trims_ids() {
        let ids = extract_firing_ids(include_bytes!("../examples/webhook_fired.json"))
            .expect("valid fixture");

        assert_eq!(ids.len(), 6);
        assert_eq!(
            ids.first().map(String::as_str),
            Some("sampleuser-session01")
        );
        assert_eq!(ids.last().map(String::as_str), Some("sampleuser-session06"));
    }

    #[test]
    fn resolved_fixture_is_a_noop() {
        let ids = extract_firing_ids(include_bytes!("../examples/webhook_resolved.json"))
            .expect("valid fixture");
        assert!(ids.is_empty());
    }

    #[test]
    fn merges_firing_alerts_and_deduplicates_ids() {
        let body = br#"{
            "alerts": [
                {"status":"firing","labels":{"stream_ids":" one, two "}},
                {"status":"resolved","labels":{"stream_ids":"ignored"}},
                {"status":"firing","labels":{"stream_ids":"two, three"}}
            ]
        }"#;

        let ids = extract_firing_ids(body).expect("valid webhook");
        assert_eq!(ids, ["one", "two", "three"]);
    }

    #[test]
    fn rejects_firing_alert_without_ids() {
        let body = br#"{"alerts":[{"status":"firing","labels":{}}]}"#;
        assert_eq!(
            extract_firing_ids(body),
            Err(WebhookError::MissingStreamIds)
        );
    }

    #[test]
    fn rejects_more_than_api_limit_before_returning_ids() {
        let stream_ids = (0..=MAX_IDS)
            .map(|index| format!("token-{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let body = serde_json::json!({
            "alerts": [{
                "status": "firing",
                "labels": { "stream_ids": stream_ids }
            }]
        })
        .to_string();

        assert_eq!(
            extract_firing_ids(body.as_bytes()),
            Err(WebhookError::TooManyIds)
        );
    }
}
