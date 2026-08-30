use crate::WebhookInputRuntime;
use std::time::Duration;

fn safe_url(url: &str) -> String {
    // Query strings are allowed for the poll itself, but may contain API keys.
    // The dashboard receives only the origin/path portion.
    url.split(['?', '#']).next().unwrap_or(url).to_owned()
}

fn consumers_for(
    bindings: Option<&Vec<crate::BlockExternalInputBinding>>,
) -> Vec<ExternalConsumerSnapshot> {
    bindings
        .into_iter()
        .flatten()
        .map(|binding| ExternalConsumerSnapshot {
            block_id: binding.block_id.to_string(),
            endpoint: binding.endpoint.to_string(),
        })
        .collect()
}

fn empty_external_value(
    value: &crate::HttpPollValueRuntime,
    consumers: Vec<ExternalConsumerSnapshot>,
) -> ExternalValueSnapshot {
    ExternalValueSnapshot {
        name: value.name.clone(),
        dpt: DptMessage::from_core(value.dpt),
        json_pointer: value.json_pointer.clone(),
        value: None,
        valid: false,
        age_ms: None,
        consumers,
        observed_at_ms: None,
    }
}

fn empty_webhook(
    source: &WebhookInputRuntime,
    consumers: Vec<ExternalConsumerSnapshot>,
) -> ExternalWebhookSnapshot {
    ExternalWebhookSnapshot {
        kind: "webhook".to_owned(),
        name: source.name.clone(),
        route: format!("/api/webhooks/{}", source.name),
        dpt: DptMessage::from_core(source.dpt),
        json_pointer: source.json_pointer.clone(),
        status: "starting".to_owned(),
        authentication_required: source.bearer_token.is_some(),
        authentication_configured: source.bearer_token.is_some(),
        last_accepted_at_ms: None,
        accepted_count: 0,
        rejected_count: 0,
        value: None,
        valid: false,
        age_ms: None,
        consumers,
        observed_at_ms: None,
    }
}

pub(crate) fn external_inputs_snapshot(runtime: &AutomationRuntime) -> ExternalInputsSnapshot {
    ExternalInputsSnapshot {
        http_polls: runtime
            .http_polls
            .iter()
            .map(|poll| ExternalPollSnapshot {
                kind: "http".to_owned(),
                name: poll.name.clone(),
                url: safe_url(&poll.url),
                interval_ms: poll.every.as_millis().try_into().unwrap_or(u64::MAX),
                status: "starting".to_owned(),
                last_attempt_at_ms: None,
                next_attempt_at_ms: None,
                last_success_at_ms: None,
                stale_at_ms: None,
                consecutive_failures: 0,
                last_error: None,
                values: poll
                    .values
                    .iter()
                    .map(|value| {
                        empty_external_value(
                            value,
                            consumers_for(runtime.http_to_inputs.get(&value.name)),
                        )
                    })
                    .collect(),
            })
            .collect(),
        webhook_inputs: runtime
            .webhook_inputs
            .iter()
            .map(|source| {
                empty_webhook(
                    source,
                    consumers_for(runtime.webhook_to_inputs.get(&source.name)),
                )
            })
            .collect(),
    }
}

fn wall_now_ms(store: &DiagnosticStore) -> Option<u64> {
    store.wall_clock_ms().and_then(|value| u64::try_from(value).ok())
}

impl DiagnosticStore {
    pub fn record_external_poll_attempt(&self, name: &str) {
        let now = wall_now_ms(self);
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(poll) = inner.external_inputs.http_polls.iter_mut().find(|poll| poll.name == name) {
            poll.last_attempt_at_ms = now;
            poll.next_attempt_at_ms = None;
            if poll.status == "starting" {
                poll.last_error = None;
            }
            self.publish_locked(&mut inner);
        }
    }

    pub fn record_external_poll_next_attempt(&self, name: &str, delay: Duration) {
        let next = wall_now_ms(self).and_then(|now| {
            now.checked_add(delay.as_millis().try_into().unwrap_or(u64::MAX))
        });
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(poll) = inner.external_inputs.http_polls.iter_mut().find(|poll| poll.name == name) {
            poll.next_attempt_at_ms = next;
            self.publish_locked(&mut inner);
        }
    }

    pub fn record_external_poll_success(
        &self,
        name: &str,
        stale_after: Duration,
        values: &[(String, TypedValue)],
    ) {
        let monotonic_now = self.now().0;
        let wall_now = wall_now_ms(self);
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(poll) = inner.external_inputs.http_polls.iter_mut().find(|poll| poll.name == name) {
            poll.status = "healthy".to_owned();
            poll.last_success_at_ms = wall_now;
            poll.stale_at_ms = wall_now.and_then(|now| now.checked_add(stale_after.as_millis().try_into().unwrap_or(u64::MAX)));
            poll.consecutive_failures = 0;
            poll.last_error = None;
            for (value_name, typed) in values {
                if let Some(value) = poll.values.iter_mut().find(|value| value.name == *value_name) {
                    value.value = Some(crate::ValueMessage::from_core(*typed));
                    value.valid = true;
                    value.observed_at_ms = Some(monotonic_now);
                    value.age_ms = Some(0);
                }
            }
            self.publish_locked(&mut inner);
        }
    }

    pub fn record_external_poll_failure(&self, name: &str, error: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(poll) = inner.external_inputs.http_polls.iter_mut().find(|poll| poll.name == name) {
            poll.status = "failing".to_owned();
            poll.consecutive_failures = poll.consecutive_failures.saturating_add(1);
            poll.last_error = Some(error.chars().take(MAX_LOGIC_ERROR).collect());
            self.publish_locked(&mut inner);
        }
    }

    pub fn record_external_poll_stale(&self, name: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(poll) = inner.external_inputs.http_polls.iter_mut().find(|poll| poll.name == name) {
            poll.status = "stale".to_owned();
            for value in &mut poll.values {
                value.value = None;
                value.valid = false;
                value.age_ms = None;
                value.observed_at_ms = None;
            }
            self.publish_locked(&mut inner);
        }
    }

    pub fn record_webhook_rejected(&self, name: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(webhook) = inner.external_inputs.webhook_inputs.iter_mut().find(|webhook| webhook.name == name) {
            webhook.rejected_count = webhook.rejected_count.saturating_add(1);
            self.publish_locked(&mut inner);
        }
    }

    pub fn record_webhook_accepted(&self, name: &str, typed: TypedValue) {
        let monotonic_now = self.now().0;
        let wall_now = wall_now_ms(self);
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(webhook) = inner.external_inputs.webhook_inputs.iter_mut().find(|webhook| webhook.name == name) {
            webhook.status = "healthy".to_owned();
            webhook.last_accepted_at_ms = wall_now;
            webhook.accepted_count = webhook.accepted_count.saturating_add(1);
            webhook.value = Some(crate::ValueMessage::from_core(typed));
            webhook.valid = true;
            webhook.age_ms = Some(0);
            webhook.observed_at_ms = Some(monotonic_now);
            self.publish_locked(&mut inner);
        }
    }
}
