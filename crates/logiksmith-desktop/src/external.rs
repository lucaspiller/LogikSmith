//! Desktop-owned HTTP polling and webhook value handling.
//!
//! This module deliberately stops at typed values.  The runtime session is
//! the sole owner of the core and decides whether a delivery is an
//! observation, trigger, or invalidation for each bound block.

use crate::{
    AutomationRuntime, HttpPollRuntime, HttpPollValueRuntime, MAX_HTTP_BODY_BYTES,
    WebhookInputRuntime, diagnostics::DiagnosticStore,
};
use futures_util::StreamExt;
use logiksmith_core::{Dpt, TypedValue, Value};
use serde_json::Value as JsonValue;
use std::{fmt, time::Duration};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time,
};

const HTTP_RETRY_LIMIT: u8 = 3;
const HTTP_RETRY_INTERVAL: Duration = Duration::from_secs(5 * 60);

fn failure_delay(retry_count: &mut u8, normal_interval: Duration) -> Duration {
    if *retry_count < HTTP_RETRY_LIMIT {
        *retry_count += 1;
        HTTP_RETRY_INTERVAL
    } else {
        *retry_count = 0;
        normal_interval
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ExternalInputUpdate {
    Observe(TypedValue),
    Trigger(TypedValue),
    Invalidate,
}

#[derive(Clone, Debug)]
pub enum ExternalInputKind {
    HttpPoll { poll: String },
    Webhook { source: String },
}

#[derive(Debug)]
pub struct ExternalInputMessage {
    pub source: String,
    pub update: ExternalInputUpdate,
    pub kind: ExternalInputKind,
    pub reply: Option<oneshot::Sender<Result<(), String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalError {
    Request(String),
    Status(u16),
    BodyTooLarge,
    InvalidJson(String),
    MissingPointer(String),
    InvalidValue(String),
    InvalidContentType,
    Unauthorized,
    QueueClosed,
}

impl fmt::Display for ExternalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(error) => write!(formatter, "request failed: {error}"),
            Self::Status(status) => write!(formatter, "HTTP status {status}"),
            Self::BodyTooLarge => formatter.write_str("body exceeds 64 KiB"),
            Self::InvalidJson(error) => write!(formatter, "invalid JSON: {error}"),
            Self::MissingPointer(pointer) => {
                write!(formatter, "JSON pointer {pointer:?} was not found")
            }
            Self::InvalidValue(error) => write!(formatter, "invalid typed value: {error}"),
            Self::InvalidContentType => {
                formatter.write_str("content type must be application/json")
            }
            Self::Unauthorized => formatter.write_str("invalid bearer token"),
            Self::QueueClosed => formatter.write_str("runtime input queue is unavailable"),
        }
    }
}

impl std::error::Error for ExternalError {}

#[derive(Debug)]
pub struct ExternalTasks {
    shutdown: watch::Sender<bool>,
    joins: Vec<JoinHandle<()>>,
}

impl ExternalTasks {
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        for mut join in self.joins {
            if time::timeout(Duration::from_secs(2), &mut join)
                .await
                .is_err()
            {
                join.abort();
                let _ = join.await;
            }
        }
    }
}

/// Start one bounded task per configured poll.  Webhooks run in the dashboard
/// server and use the same sender; they therefore do not need a background
/// task here.
pub fn spawn_http_polls(
    automation: &AutomationRuntime,
    sender: mpsc::Sender<ExternalInputMessage>,
    store: DiagnosticStore,
) -> ExternalTasks {
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let mut joins = Vec::new();
    for poll in automation.http_polls.clone() {
        let sender = sender.clone();
        let shutdown = shutdown_receiver.clone();
        let store = store.clone();
        joins.push(tokio::spawn(async move {
            poll_loop(poll, sender, shutdown, store).await;
        }));
    }
    ExternalTasks {
        shutdown: shutdown_sender,
        joins,
    }
}

async fn poll_loop(
    poll: HttpPollRuntime,
    sender: mpsc::Sender<ExternalInputMessage>,
    mut shutdown: watch::Receiver<bool>,
    store: DiagnosticStore,
) {
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(target: "logiksmith.external", poll = %poll.name, error = %error, "unable to create HTTP client");
            return;
        }
    };
    let mut next_poll = time::Instant::now();
    let mut retry_count = 0;
    let mut stale_at: Option<time::Instant> = None;
    let mut stale_sent = false;
    let mut values: std::collections::HashMap<String, TypedValue> =
        std::collections::HashMap::new();
    loop {
        let now = time::Instant::now();
        let deadline = stale_at
            .filter(|_| !stale_sent)
            .map_or(next_poll, |stale| stale.min(next_poll));
        let wait = deadline.saturating_duration_since(now);
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return; }
            },
            _ = time::sleep(wait) => {}
        }
        let now = time::Instant::now();
        if !stale_sent && stale_at.is_some_and(|deadline| now >= deadline) {
            for value in &poll.values {
                if sender
                    .send(ExternalInputMessage {
                        source: value.name.clone(),
                        update: ExternalInputUpdate::Invalidate,
                        kind: ExternalInputKind::HttpPoll {
                            poll: poll.name.clone(),
                        },
                        reply: None,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
            }
            store.record_external_poll_stale(&poll.name);
            values.clear();
            stale_sent = true;
        }
        if now < next_poll {
            continue;
        }
        store.record_external_poll_attempt(&poll.name);
        let next_delay = match fetch_poll(&client, &poll).await {
            Ok(extracted) => {
                retry_count = 0;
                let accepted_at = time::Instant::now();
                stale_at = Some(accepted_at + poll.stale_after);
                stale_sent = false;
                let diagnostics_values: Vec<_> = extracted
                    .iter()
                    .map(|(value, typed)| (value.name.clone(), *typed))
                    .collect();
                store.record_external_poll_success(
                    &poll.name,
                    poll.stale_after,
                    &diagnostics_values,
                );
                for (value, typed) in extracted {
                    let update = match values.insert(value.name.clone(), typed) {
                        Some(previous) if previous == typed => ExternalInputUpdate::Observe(typed),
                        _ => ExternalInputUpdate::Trigger(typed),
                    };
                    if sender
                        .send(ExternalInputMessage {
                            source: value.name.clone(),
                            update,
                            kind: ExternalInputKind::HttpPoll {
                                poll: poll.name.clone(),
                            },
                            reply: None,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                poll.every
            }
            Err(error) => {
                // reqwest may include the full URL in request errors. Keep
                // query strings (which can contain credentials) out of both
                // logs and the dashboard error projection.
                let diagnostic = match &error {
                    ExternalError::Request(_) => "request failed".to_owned(),
                    _ => error.to_string(),
                };
                store.record_external_poll_failure(&poll.name, &diagnostic);
                tracing::warn!(target: "logiksmith.external", poll = %poll.name, error = %diagnostic, "HTTP poll failed");
                failure_delay(&mut retry_count, poll.every)
            }
        };
        next_poll = time::Instant::now() + next_delay;
        store.record_external_poll_next_attempt(&poll.name, next_delay);
    }
}

async fn fetch_poll(
    client: &reqwest::Client,
    poll: &HttpPollRuntime,
) -> Result<Vec<(HttpPollValueRuntime, TypedValue)>, ExternalError> {
    let mut request = client.get(&poll.url).timeout(poll.timeout);
    for (name, value) in &poll.headers {
        request = request.header(name, value);
    }
    let response = request
        .send()
        .await
        .map_err(|error| ExternalError::Request(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(ExternalError::Status(status.as_u16()));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    {
        return Err(ExternalError::InvalidContentType);
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| ExternalError::Request(error.to_string()))?;
        if body.len().saturating_add(chunk.len()) > MAX_HTTP_BODY_BYTES {
            return Err(ExternalError::BodyTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    let json: JsonValue = serde_json::from_slice(&body)
        .map_err(|error| ExternalError::InvalidJson(error.to_string()))?;
    poll.values
        .iter()
        .map(|value| {
            let selected = json
                .pointer(&value.json_pointer)
                .ok_or_else(|| ExternalError::MissingPointer(value.json_pointer.clone()))?;
            let typed = typed_value(value.dpt, selected).map_err(ExternalError::InvalidValue)?;
            Ok((value.clone(), typed))
        })
        .collect()
}

pub fn parse_webhook_value(
    source: &WebhookInputRuntime,
    body: &[u8],
) -> Result<TypedValue, ExternalError> {
    if body.len() > MAX_HTTP_BODY_BYTES {
        return Err(ExternalError::BodyTooLarge);
    }
    let json: JsonValue = serde_json::from_slice(body)
        .map_err(|error| ExternalError::InvalidJson(error.to_string()))?;
    let selected = json
        .pointer(&source.json_pointer)
        .ok_or_else(|| ExternalError::MissingPointer(source.json_pointer.clone()))?;
    typed_value(source.dpt, selected).map_err(ExternalError::InvalidValue)
}

fn typed_value(dpt: Dpt, value: &JsonValue) -> Result<TypedValue, String> {
    if dpt.is_bool() {
        return value
            .as_bool()
            .map(TypedValue::bool)
            .ok_or_else(|| "expected a JSON boolean".to_owned());
    }
    if dpt.is_percent() {
        let number = value
            .as_u64()
            .ok_or_else(|| "expected an integer from 0 through 100".to_owned())?;
        let number = u8::try_from(number)
            .map_err(|_| "expected an integer from 0 through 100".to_owned())?;
        return TypedValue::percent(number).map_err(|error| error.to_string());
    }
    if dpt.is_temperature() {
        let number = value
            .as_number()
            .ok_or_else(|| "expected a JSON number in degrees Celsius".to_owned())?;
        let centi = parse_centi_degrees(&number.to_string())?;
        return TypedValue::new(dpt, Value::Temperature(i32::from(centi)))
            .map_err(|error| error.to_string());
    }
    Err(format!("unsupported DPT {dpt}"))
}

fn parse_centi_degrees(value: &str) -> Result<i32, String> {
    let (negative, value) = value
        .strip_prefix('-')
        .map_or((false, value), |value| (true, value));
    let (whole, fractional) = value
        .split_once('.')
        .map_or((value, ""), |(whole, fractional)| (whole, fractional));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fractional.len() > 2
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("must have at most two decimal places".to_owned());
    }
    let whole = whole
        .parse::<i32>()
        .map_err(|_| "is out of range".to_owned())?;
    let fraction = match fractional.len() {
        0 => 0,
        1 => fractional.parse::<i32>().unwrap_or(0) * 10,
        _ => fractional.parse::<i32>().unwrap_or(0),
    };
    let centi = whole
        .checked_mul(100)
        .and_then(|whole| whole.checked_add(fraction))
        .ok_or_else(|| "is out of range".to_owned())?;
    let centi = if negative {
        centi
            .checked_neg()
            .ok_or_else(|| "is out of range".to_owned())?
    } else {
        centi
    };
    Ok(centi)
}

pub fn webhook_authorized(source: &WebhookInputRuntime, authorization: Option<&str>) -> bool {
    match &source.bearer_token {
        None => true,
        Some(expected) => authorization
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|value| constant_time_equal(value.as_bytes(), expected.as_bytes())),
    }
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= usize::from(
            left.get(index).copied().unwrap_or_default()
                ^ right.get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_meteo_values_are_extracted_with_strict_temperature_precision() {
        let document = serde_json::json!({
            "daily": {
                "temperature_2m_max": [21.75],
                "temperature_2m_min": [8.25]
            }
        });
        let max = HttpPollValueRuntime {
            name: "today_temperature_max".to_owned(),
            dpt: Dpt::TEMPERATURE,
            json_pointer: "/daily/temperature_2m_max/0".to_owned(),
        };
        let min = HttpPollValueRuntime {
            name: "today_temperature_min".to_owned(),
            dpt: Dpt::TEMPERATURE,
            json_pointer: "/daily/temperature_2m_min/0".to_owned(),
        };
        let json = serde_json::to_vec(&document).unwrap();
        let poll = HttpPollRuntime {
            name: "forecast".to_owned(),
            url: "http://127.0.0.1/forecast".to_owned(),
            every: Duration::from_secs(3600),
            timeout: Duration::from_secs(5),
            stale_after: Duration::from_secs(7200),
            headers: Vec::new(),
            values: vec![max, min],
        };
        // The parser is exercised through the same poll extraction path; a
        // response with both values is all-or-nothing.
        let selected_max = document.pointer(&poll.values[0].json_pointer).unwrap();
        assert_eq!(
            typed_value(Dpt::TEMPERATURE, selected_max)
                .unwrap()
                .temperature_centi(),
            Some(2175)
        );
        let selected_min = document.pointer(&poll.values[1].json_pointer).unwrap();
        assert_eq!(
            typed_value(Dpt::TEMPERATURE, selected_min)
                .unwrap()
                .temperature_centi(),
            Some(825)
        );
        assert!(typed_value(Dpt::TEMPERATURE, &serde_json::json!(1.234)).is_err());
        assert!(!json.is_empty());
    }

    #[test]
    fn webhook_auth_and_scalar_conversion_are_exact() {
        let source = WebhookInputRuntime {
            name: "override".to_owned(),
            dpt: Dpt::BOOL,
            json_pointer: "/enabled".to_owned(),
            bearer_token: Some("secret".to_owned()),
        };
        assert!(webhook_authorized(&source, Some("Bearer secret")));
        assert!(!webhook_authorized(&source, Some("Bearer wrong")));
        assert!(!webhook_authorized(&source, None));
        assert_eq!(
            parse_webhook_value(&source, br#"{"enabled":true}"#)
                .unwrap()
                .value(),
            Value::Bool(true)
        );
        assert!(parse_webhook_value(&source, br#"{"enabled":1}"#).is_err());
    }

    #[test]
    fn external_json_conversion_covers_signed_temperatures_and_scalar_bounds() {
        assert_eq!(parse_centi_degrees("-4.2"), Ok(-420));
        assert_eq!(parse_centi_degrees("0"), Ok(0));
        assert!(parse_centi_degrees("4.321").is_err());
        assert!(parse_centi_degrees("-").is_err());

        assert_eq!(
            typed_value(Dpt::PERCENT, &serde_json::json!(100))
                .unwrap()
                .value(),
            Value::Percent(100)
        );
        assert!(typed_value(Dpt::PERCENT, &serde_json::json!(101)).is_err());
        assert!(typed_value(Dpt::BOOL, &serde_json::json!(1)).is_err());
    }

    #[test]
    fn failed_polls_use_three_fixed_retries_then_normal_interval() {
        let mut retry_count = 0;
        let normal_interval = Duration::from_secs(3600);

        for expected_count in 1..=HTTP_RETRY_LIMIT {
            let delay = failure_delay(&mut retry_count, normal_interval);
            assert_eq!(delay, HTTP_RETRY_INTERVAL);
            assert_eq!(retry_count, expected_count);
        }

        let delay = failure_delay(&mut retry_count, normal_interval);
        assert_eq!(delay, normal_interval);
        assert_eq!(retry_count, 0);

        let delay = failure_delay(&mut retry_count, normal_interval);
        assert_eq!(delay, HTTP_RETRY_INTERVAL);
        assert_eq!(retry_count, 1);
    }
}
