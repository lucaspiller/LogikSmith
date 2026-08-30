fn trigger_record(
    trigger: &Trigger,
    document_revision: u64,
    schedule_handling: Option<ScheduleHandling>,
) -> LogicalTriggerRecord {
    match trigger {
        Trigger::Input(trigger) => LogicalTriggerRecord {
            trigger_type: "input".to_owned(),
            endpoint: trigger.endpoint.to_string(),
            dpt: DptMessage::from_core(trigger.value.dpt),
            value: ValueMessage::from_core(trigger.value),
            previous: trigger.previous.map(ValueMessage::from_core),
            changed: trigger.changed,
            rising: trigger.rising,
            falling: trigger.falling,
            name: None,
            scheduled_at_ms: None,
            due_at_ms: None,
            fired_at_ms: None,
            late_by_ms: None,
            scheduled_logic_revision: None,
            kind: None,
            scheduled_for_utc_ms: None,
            detected_at_utc_ms: None,
            handled_at_utc_ms: None,
            queue_delay_ms: None,
            coalesced_count: None,
            structural_revision: None,
        },
        Trigger::Timer(trigger) => LogicalTriggerRecord {
            trigger_type: "timer".to_owned(),
            endpoint: String::new(),
            dpt: DptMessage {
                major: 0,
                subtype: 0,
            },
            value: ValueMessage::Bool(BoolValueMessage {
                kind: "bool".to_owned(),
                value: false,
            }),
            previous: None,
            changed: false,
            rising: false,
            falling: false,
            name: Some(trigger.name.to_string()),
            scheduled_at_ms: Some(trigger.scheduled_at.0),
            due_at_ms: Some(trigger.due_at.0),
            fired_at_ms: Some(trigger.fired_at.0),
            late_by_ms: Some(trigger.fired_at.0.saturating_sub(trigger.due_at.0)),
            scheduled_logic_revision: Some(document_revision),
            kind: None,
            scheduled_for_utc_ms: None,
            detected_at_utc_ms: None,
            handled_at_utc_ms: None,
            queue_delay_ms: None,
            coalesced_count: None,
            structural_revision: None,
        },
        Trigger::Schedule(trigger) => {
            let handling = schedule_handling.unwrap_or(ScheduleHandling {
                handled_at_utc_ms: trigger.detected_at_utc_ms,
                coalesced_count: 0,
            });
            LogicalTriggerRecord {
                trigger_type: "schedule".to_owned(),
                endpoint: String::new(),
                dpt: DptMessage {
                    major: 0,
                    subtype: 0,
                },
                value: ValueMessage::Bool(BoolValueMessage {
                    kind: "bool".to_owned(),
                    value: false,
                }),
                previous: None,
                changed: false,
                rising: false,
                falling: false,
                name: Some(trigger.name.to_string()),
                scheduled_at_ms: None,
                due_at_ms: None,
                fired_at_ms: None,
                late_by_ms: Some(
                    trigger
                        .detected_at_utc_ms
                        .saturating_sub(trigger.scheduled_for_utc_ms)
                        .unsigned_abs(),
                ),
                scheduled_logic_revision: None,
                kind: Some(schedule_kind(&trigger.kind)),
                scheduled_for_utc_ms: Some(trigger.scheduled_for_utc_ms),
                detected_at_utc_ms: Some(trigger.detected_at_utc_ms),
                handled_at_utc_ms: Some(handling.handled_at_utc_ms),
                queue_delay_ms: Some(
                    handling
                        .handled_at_utc_ms
                        .saturating_sub(trigger.detected_at_utc_ms)
                        .unsigned_abs(),
                ),
                coalesced_count: Some(handling.coalesced_count),
                structural_revision: Some(trigger.structural_revision),
            }
        }
    }
}

/// Host-measured schedule handling facts that the core trigger does not own:
/// the wall-clock instant handling started and the number of coalesced
/// occurrences skipped by the latest-only delivery policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduleHandling {
    pub handled_at_utc_ms: i64,
    pub coalesced_count: u64,
}

fn schedule_kind(kind: &logiksmith_core::ScheduleKind) -> String {
    match kind {
        logiksmith_core::ScheduleKind::Fixed => "fixed".to_owned(),
        logiksmith_core::ScheduleKind::Interval => "interval".to_owned(),
        logiksmith_core::ScheduleKind::Astronomical => "astronomical".to_owned(),
    }
}

pub fn simulation_response(
    execution: &Execution,
    duration_us: u64,
    logic_revision: u64,
    automation: &AutomationRuntime,
) -> SimulationResponse {
    let signal_effects = signal_effect_records(&execution.signal_effects);
    let (status, effects, transition, error) = match &execution.outcome {
        Ok(effects) => (
            LogicExecutionStatus::Succeeded,
            effects
                .outputs
                .iter()
                .filter_map(|effect| effect_record(effect, automation))
                .collect(),
            Some({
                let mut transition = transition_record(effects, automation);
                transition.signal_effects = signal_effects.clone();
                transition
            }),
            None,
        ),
        Err(error) => (
            LogicExecutionStatus::Failed,
            Vec::new(),
            None,
            Some(logic_error_record(error)),
        ),
    };
    SimulationResponse {
        block_id: automation
            .blocks
            .first()
            .map(|block| block.id.clone())
            .unwrap_or_default(),
        logic_revision,
        duration_us,
        status,
        trigger: trigger_record(&execution.trigger, logic_revision, None),
        time_context: time_context_record(&execution.time_context),
        inputs: execution.inputs.iter().map(input_snapshot_record).collect(),
        state_before: state_record(&execution.state_before),
        state_after: state_record(&execution.state_after),
        transition: transition.clone(),
        pending_timers: execution
            .pending_timers
            .iter()
            .map(|timer| pending_timer_record(timer, logic_revision))
            .collect(),
        effects,
        signal_effects,
        eligible_consumers: execution
            .eligible_consumers
            .iter()
            .map(signal_consumer_record)
            .collect(),
        timer_effects: transition
            .as_ref()
            .map(|transition| transition.timers.clone())
            .unwrap_or_default(),
        error,
    }
}

pub fn simulation_response_for_block(
    tagged: &BlockExecution,
    duration_us: u64,
    logic_revision: u64,
    automation: &AutomationRuntime,
) -> SimulationResponse {
    let execution = &tagged.execution;
    let signal_effects = signal_effect_records(&execution.signal_effects);
    let (status, effects, transition, error) = match &execution.outcome {
        Ok(effects) => (
            LogicExecutionStatus::Succeeded,
            effects
                .outputs
                .iter()
                .filter_map(|effect| effect_record_for_block(effect, automation, &tagged.block_id))
                .collect(),
            Some({
                let mut transition =
                    transition_record_for_block(effects, automation, &tagged.block_id);
                transition.signal_effects = signal_effects.clone();
                transition
            }),
            None,
        ),
        Err(error) => (
            LogicExecutionStatus::Failed,
            Vec::new(),
            None,
            Some(logic_error_record(error)),
        ),
    };
    SimulationResponse {
        block_id: tagged.block_id.to_string(),
        logic_revision,
        duration_us,
        status,
        trigger: trigger_record(&execution.trigger, logic_revision, None),
        time_context: time_context_record(&execution.time_context),
        inputs: execution.inputs.iter().map(input_snapshot_record).collect(),
        state_before: state_record(&execution.state_before),
        state_after: state_record(&execution.state_after),
        transition: transition.clone(),
        pending_timers: execution
            .pending_timers
            .iter()
            .map(|timer| pending_timer_record(timer, logic_revision))
            .collect(),
        effects,
        signal_effects,
        eligible_consumers: execution
            .eligible_consumers
            .iter()
            .map(signal_consumer_record)
            .collect(),
        timer_effects: transition
            .as_ref()
            .map(|transition| transition.timers.clone())
            .unwrap_or_default(),
        error,
    }
}

fn input_snapshot_record(input: &logiksmith_core::InputSnapshot) -> LogicalInputSnapshot {
    LogicalInputSnapshot {
        endpoint: input.endpoint.to_string(),
        dpt: DptMessage::from_core(input.dpt),
        value: input.value.map(ValueMessage::from_core),
        valid: input.valid,
        age_ms: input.age_ms,
    }
}

fn effect_record(
    effect: &OutputEffect,
    automation: &AutomationRuntime,
) -> Option<LogicalEffectRecord> {
    let block_id = automation.blocks.first().map(|block| block.id.as_str())?;
    effect_record_for_block(effect, automation, &block_id.parse().ok()?)
}

fn effect_record_for_block(
    effect: &OutputEffect,
    automation: &AutomationRuntime,
    block_id: &logiksmith_core::BlockId,
) -> Option<LogicalEffectRecord> {
    let endpoint = &effect.endpoint;
    let value = effect.value;
    Some(LogicalEffectRecord {
        block_id: block_id.to_string(),
        endpoint: endpoint.to_string(),
        destination: automation
            .output_to_address
            .get(&(block_id.to_string(), endpoint.clone()))?
            .to_string(),
        dpt: DptMessage::from_core(value.dpt),
        value: ValueMessage::from_core(value),
    })
}

fn state_value_record(value: &StateValue) -> StateValueRecord {
    let (kind, value) = match value {
        StateValue::Bool(value) => ("bool", JsonValue::Bool(*value)),
        StateValue::Integer(value) => ("integer", JsonValue::Number((*value).into())),
        StateValue::Number(value) => (
            "number",
            serde_json::Number::from_f64(*value)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
        ),
        StateValue::String(value) => ("string", JsonValue::String(value.clone())),
    };
    StateValueRecord {
        kind: kind.to_owned(),
        value,
    }
}

fn state_record(state: &logiksmith_core::TransientState) -> BTreeMap<String, StateValueRecord> {
    state
        .iter()
        .map(|(key, value)| (key.clone(), state_value_record(value)))
        .collect()
}

fn pending_timer_record(
    timer: &logiksmith_core::PendingTimer,
    document_revision: u64,
) -> PendingTimerRecord {
    PendingTimerRecord {
        name: timer.name.to_string(),
        scheduled_at_ms: timer.scheduled_at.0,
        due_at_ms: timer.due_at.0,
        logic_revision: document_revision,
    }
}

fn transition_record(
    transition: &logiksmith_core::Transition,
    automation: &AutomationRuntime,
) -> LogicalTransitionRecord {
    let Some(block_id) = automation
        .blocks
        .first()
        .and_then(|block| block.id.parse::<logiksmith_core::BlockId>().ok())
    else {
        return LogicalTransitionRecord {
            state: state_record(&transition.state),
            effects: Vec::new(),
            signal_effects: Vec::new(),
            timers: transition_timer_records(transition),
        };
    };
    transition_record_for_block(transition, automation, &block_id)
}

fn transition_record_for_block(
    transition: &logiksmith_core::Transition,
    automation: &AutomationRuntime,
    block_id: &logiksmith_core::BlockId,
) -> LogicalTransitionRecord {
    LogicalTransitionRecord {
        state: state_record(&transition.state),
        effects: transition
            .outputs
            .iter()
            .filter_map(|effect| effect_record_for_block(effect, automation, block_id))
            .collect(),
        signal_effects: Vec::new(),
        timers: transition_timer_records(transition),
    }
}

fn transition_timer_records(
    transition: &logiksmith_core::Transition,
) -> Vec<LogicalTimerEffectRecord> {
    transition
        .timers
        .iter()
        .map(|effect| {
            let (action, after_ms, previous_due_at_ms, due_at_ms) = match effect.action {
                TimerAction::Scheduled { after_ms, due_at } => {
                    ("scheduled", Some(after_ms), None, Some(due_at.0))
                }
                TimerAction::Replaced {
                    previous_due_at,
                    after_ms,
                    due_at,
                } => (
                    "replaced",
                    Some(after_ms),
                    Some(previous_due_at.0),
                    Some(due_at.0),
                ),
                TimerAction::Cancelled { previous_due_at } => {
                    ("cancelled", None, Some(previous_due_at.0), None)
                }
                TimerAction::CancelNoop => ("cancel_noop", None, None, None),
            };
            LogicalTimerEffectRecord {
                name: effect.name.to_string(),
                action: action.to_owned(),
                after_ms,
                previous_due_at_ms,
                due_at_ms,
            }
        })
        .collect()
}

fn logic_error_record(error: &logiksmith_core::LogicError) -> LogicErrorRecord {
    let mut message = error.message().to_owned();
    if message.len() > MAX_LOGIC_ERROR {
        let end = (0..=MAX_LOGIC_ERROR)
            .rev()
            .find(|index| message.is_char_boundary(*index))
            .unwrap_or(0);
        message.truncate(end);
    }
    LogicErrorRecord {
        category: error.category().to_owned(),
        message,
        line: error.line().and_then(|line| u32::try_from(line).ok()),
    }
}
/// Title-cases one weekday token for display (`mon` -> `Mon`).
fn weekday_token(token: &str) -> String {
    let mut characters = token.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

/// Builds the read-only rule display for one document schedule. The summary
/// mirrors the canonical configuration the user saved.
fn schedule_config_snapshot(schedule: &crate::AutomationSchedule) -> ScheduleConfigSnapshot {
    let mut summary = String::new();
    match schedule.kind.as_str() {
        "fixed" => {
            if let Some(at) = &schedule.at {
                summary.push_str(&format!("at {at}"));
            }
        }
        "interval" => {
            if let Some(every) = &schedule.every {
                summary.push_str(&format!("every {every}"));
            }
            if let Some(offset) = &schedule.offset
                && offset != "0s"
            {
                summary.push_str(&format!(" offset {offset}"));
            }
        }
        "astronomical" => {
            if let Some(anchor) = &schedule.anchor {
                summary.push_str(anchor);
            }
            if let Some(offset) = &schedule.offset {
                summary.push_str(&format!(" {offset}"));
            }
        }
        other => {
            summary.push_str(other);
        }
    }
    if let Some(weekdays) = &schedule.weekdays {
        if !weekdays.is_empty() {
            summary.push_str(" ");
            summary.push_str(
                &weekdays
                    .iter()
                    .map(|token| weekday_token(token))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
    }
    ScheduleConfigSnapshot {
        name: schedule.name.clone(),
        enabled: schedule.enabled,
        rule: ScheduleRuleSnapshot {
            kind: schedule.kind.clone(),
            summary: summary.trim().to_owned(),
        },
    }
}

fn format_utc_ms(utc_ms: i64, timezone: &str) -> Option<(String, i64)> {
    use jiff::{Timestamp, tz::TimeZone};
    let timestamp = Timestamp::from_millisecond(utc_ms).ok()?;
    let zoned = timestamp.to_zoned(TimeZone::get(timezone).ok()?);
    let datetime = zoned.datetime();
    let offset = zoned.offset().seconds();
    Some((
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            datetime.year(),
            datetime.month(),
            datetime.day(),
            datetime.hour(),
            datetime.minute(),
            datetime.second(),
        ),
        i64::from(offset),
    ))
}

/// Converts a captured core time context into the immutable browser record.
fn time_context_record(context: &logiksmith_core::TimeContext) -> TimeContextRecord {
    TimeContextRecord {
        now: date_time_value_record(&context.now),
        sun: SunContextRecord {
            dawn: date_time_value_record(&context.sun.dawn),
            sunrise: date_time_value_record(&context.sun.sunrise),
            sunset: date_time_value_record(&context.sun.sunset),
            dusk: date_time_value_record(&context.sun.dusk),
            elevation_degrees: context.sun.elevation_degrees,
            azimuth_degrees: context.sun.azimuth_degrees,
        },
    }
}

fn date_time_value_record(value: &logiksmith_core::DateTimeValue) -> DateTimeValueRecord {
    DateTimeValueRecord {
        available: value.available,
        year: value.year,
        month: value.month,
        day: value.day,
        hour: value.hour,
        minute: value.minute,
        second: value.second,
        weekday: value.weekday.as_ref().map(ToString::to_string),
    }
}

/// Builds the initial site card from the configured site facts before any
/// live clock sample exists.
fn site_time_snapshot(site: &logiksmith_core::SiteTimeConfig) -> SiteTimeSnapshot {
    let coordinates = site
        .coordinates
        .as_ref()
        .map(|coordinates| CoordinatesSnapshot {
            latitude: coordinates.latitude,
            longitude: coordinates.longitude,
        });
    SiteTimeSnapshot {
        timezone: site.timezone.as_str().to_owned(),
        local_time: None,
        utc_offset: None,
        coordinates,
        astronomy: "unavailable".to_owned(),
        astronomy_reason: Some(if coordinates.is_some() {
            "waiting for the first wall-clock sample".to_owned()
        } else {
            "no coordinates configured".to_owned()
        }),
        dawn: None,
        sunrise: None,
        sunset: None,
        dusk: None,
        clock_ok: false,
        scheduler_ok: false,
    }
}

/// Builds the site card from a fresh paired clock sample. Solar events and
/// the local projection come from the portable core time context; the wall
/// clock and UTC offset are formatted here with the configured IANA zone.
pub fn site_time_snapshot_live(
    site: &logiksmith_core::SiteTimeConfig,
    sample: &ClockSample,
) -> SiteTimeSnapshot {
    let context = TimeContext::capture(site, sample.utc_unix_ms);
    let coordinates = site
        .coordinates
        .as_ref()
        .map(|coordinates| CoordinatesSnapshot {
            latitude: coordinates.latitude,
            longitude: coordinates.longitude,
        });
    let timezone = site.timezone.as_str();
    let (local_time, utc_offset) = match sample
        .utc_unix_ms
        .and_then(|utc_ms| format_utc_ms(utc_ms, timezone))
    {
        Some((local, offset)) => (Some(local), Some(offset)),
        None => (None, None),
    };
    let format_event = |value: &logiksmith_core::DateTimeValue| -> Option<String> {
        if !value.available {
            return None;
        }
        Some(format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            value.year?, value.month?, value.day?, value.hour?, value.minute?, value.second?
        ))
    };
    let solar_available = coordinates.is_some() && sample.utc_unix_ms.is_some();
    SiteTimeSnapshot {
        timezone: timezone.to_owned(),
        local_time,
        utc_offset,
        coordinates,
        astronomy: if solar_available {
            "available".to_owned()
        } else {
            "unavailable".to_owned()
        },
        astronomy_reason: if solar_available {
            None
        } else if coordinates.is_none() {
            Some("no coordinates configured".to_owned())
        } else {
            Some("wall clock unavailable".to_owned())
        },
        dawn: format_event(&context.sun.dawn),
        sunrise: format_event(&context.sun.sunrise),
        sunset: format_event(&context.sun.sunset),
        dusk: format_event(&context.sun.dusk),
        clock_ok: sample.utc_unix_ms.is_some(),
        scheduler_ok: sample.utc_unix_ms.is_some(),
    }
}

/// Maps one core scheduler status into the browser feed row the session
/// pushes after every poll or structural restart.
pub fn schedule_status_feed(status: &ScheduleStatus) -> ScheduleStatusFeed {
    let (clock_error, unavailable_reason) = match &status.status {
        logiksmith_core::ScheduleStatusKind::ClockError => (true, None),
        logiksmith_core::ScheduleStatusKind::Unavailable { reason } => {
            (false, Some(reason.clone()))
        }
        _ => (false, None),
    };
    ScheduleStatusFeed {
        block_id: status.block_id.to_string(),
        name: status.name.to_string(),
        clock_error,
        unavailable_reason,
        next_occurrence_utc_ms: status.next_occurrence_utc_ms,
    }
}

/// Builds the read-only occurrence preview for a schedule trigger simulation
/// without a selected instant. The preview itself mutates nothing.
pub fn schedule_preview_response(
    block_id: &str,
    schedule: &crate::AutomationSchedule,
    occurrences: &[ScheduleOccurrence],
    timezone: &str,
) -> SchedulePreviewResponse {
    SchedulePreviewResponse {
        block_id: block_id.to_owned(),
        schedule: schedule.name.clone(),
        rule: schedule_config_snapshot(schedule).rule,
        occurrences: occurrences
            .iter()
            .filter_map(|occurrence| schedule_occurrence_snapshot(occurrence, timezone))
            .collect(),
    }
}

fn schedule_occurrence_snapshot(
    occurrence: &ScheduleOccurrence,
    timezone: &str,
) -> Option<ScheduleOccurrenceSnapshot> {
    use jiff::{Timestamp, tz::TimeZone};
    let timestamp = Timestamp::from_millisecond(occurrence.utc_ms).ok()?;
    let zoned = timestamp.to_zoned(TimeZone::get(timezone).ok()?);
    let datetime = zoned.datetime();
    Some(ScheduleOccurrenceSnapshot {
        utc_ms: occurrence.utc_ms,
        local: format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            datetime.year(),
            datetime.month(),
            datetime.day(),
            datetime.hour(),
            datetime.minute(),
            datetime.second(),
        ),
        utc_offset: i64::from(zoned.offset().seconds()),
        weekday: Some(weekday_name(zoned.datetime().weekday())),
    })
}

fn weekday_name(weekday: jiff::civil::Weekday) -> String {
    match weekday {
        jiff::civil::Weekday::Monday => "Monday",
        jiff::civil::Weekday::Tuesday => "Tuesday",
        jiff::civil::Weekday::Wednesday => "Wednesday",
        jiff::civil::Weekday::Thursday => "Thursday",
        jiff::civil::Weekday::Friday => "Friday",
        jiff::civil::Weekday::Saturday => "Saturday",
        jiff::civil::Weekday::Sunday => "Sunday",
    }
    .to_owned()
}

fn automation_snapshot(runtime: &AutomationRuntime) -> AutomationSnapshot {
    let Some(block) = runtime.blocks.first() else {
        return AutomationSnapshot {
            inputs: Vec::new(),
            outputs: Vec::new(),
            knx_bindings: Vec::new(),
            signal_bindings: Vec::new(),
            logic: LogicSourceSnapshot {
                source: String::new(),
            },
        };
    };
    block_automation_snapshot(runtime, block)
}

fn block_automation_snapshot(
    runtime: &AutomationRuntime,
    block: &crate::BlockRuntime,
) -> AutomationSnapshot {
    let source = runtime
        .document
        .blocks
        .iter()
        .find(|candidate| candidate.id == block.id)
        .map(|candidate| candidate.source.as_str())
        .unwrap_or_default();
    block_automation_snapshot_with_source(block, source)
}

fn block_automation_snapshot_with_source(
    block: &crate::BlockRuntime,
    source: &str,
) -> AutomationSnapshot {
    let endpoint = |name: &EndpointName, dpt: Dpt| {
        let signal = block.endpoint_to_signal.get(name).cloned();
        EndpointSnapshot {
            name: name.to_string(),
            dpt: DptMessage::from_core(dpt),
            binding_kind: if block.endpoint_to_address.contains_key(name) {
                "knx"
            } else if signal.is_some() {
                "signal"
            } else {
                "unbound"
            }
            .to_owned(),
            signal,
        }
    };
    let inputs = block
        .engine_config
        .endpoints
        .iter()
        .filter(|item| item.direction == EndpointDirection::Input)
        .map(|item| endpoint(&item.name, item.dpt))
        .collect();
    let outputs = block
        .engine_config
        .endpoints
        .iter()
        .filter(|item| item.direction == EndpointDirection::Output)
        .map(|item| endpoint(&item.name, item.dpt))
        .collect();
    let mut knx_bindings: Vec<_> = block
        .endpoint_to_address
        .iter()
        .map(|(endpoint, address)| BindingSnapshot {
            endpoint: endpoint.to_string(),
            group_address: address.to_string(),
        })
        .collect();
    knx_bindings.sort_by(|left, right| left.endpoint.cmp(&right.endpoint));
    let mut signal_bindings: Vec<_> = block
        .endpoint_to_signal
        .iter()
        .map(|(endpoint, signal)| SignalBindingSnapshot {
            endpoint: endpoint.to_string(),
            signal: signal.clone(),
        })
        .collect();
    signal_bindings.sort_by(|left, right| left.endpoint.cmp(&right.endpoint));
    AutomationSnapshot {
        inputs,
        outputs,
        knx_bindings,
        signal_bindings,
        logic: LogicSourceSnapshot {
            source: source.to_owned(),
        },
    }
}

/// Refreshes only active document-derived projections. Structural saved
/// documents remain separate until restart, while source-only activation can
/// update the running source immediately.
fn update_active_document_projection_locked(
    inner: &mut Inner,
    document: &crate::AutomationDocument,
) {
    let mut runtime_view = None;
    if let Some(first) = inner.block_order.first()
        && let Some(block) = inner.block_automation.get(first)
    {
        runtime_view = Some(block.clone());
    }
    for candidate in &document.blocks {
        if let Some(block) = inner.blocks.get_mut(&candidate.id) {
            block.source = candidate.source.clone();
        }
        if let Some(automation) = inner.block_automation.get_mut(&candidate.id) {
            automation.logic.source = candidate.source.clone();
        }
        if let Some(configs) = inner.block_schedules.get_mut(&candidate.id) {
            *configs = candidate
                .schedules
                .iter()
                .map(schedule_config_snapshot)
                .collect();
        }
    }
    // The compatibility top-level view is the first declared block. It is
    // safe to update its source without rebuilding endpoint maps because the
    // only hot document changes are source/enabled fields.
    if let Some(first_id) = inner.block_order.first()
        && let Some(first_document) = document.blocks.iter().find(|block| &block.id == first_id)
    {
        inner.automation.logic.source = first_document.source.clone();
    } else if let Some(runtime_view) = runtime_view {
        inner.automation = runtime_view;
    }
}
fn block_schedule_snapshots(
    inner: &Inner,
    block_id: &str,
    state: &BlockDiagnosticState,
) -> Vec<ScheduleSnapshot> {
    let Some(configs) = inner.block_schedules.get(block_id) else {
        return Vec::new();
    };
    let timezone = &inner.site_time.timezone;
    let wall_now_ms = inner
        .last_clock_sample
        .and_then(|sample| sample.utc_unix_ms);
    configs
        .iter()
        .map(|config| {
            let feed = inner
                .schedule_status
                .get(&(block_id.to_owned(), config.name.clone()));
            let paused = !state.active_enabled || !config.enabled;
            let (status, unavailable_reason) = if paused {
                ("paused", None)
            } else if feed.is_some_and(|feed| feed.clock_error) {
                ("clock_error", None)
            } else if let Some(reason) = feed.and_then(|feed| feed.unavailable_reason.clone()) {
                ("unavailable", Some(reason))
            } else {
                ("active", None)
            };
            let next_utc = feed.and_then(|feed| feed.next_occurrence_utc_ms);
            let (next_occurrence, utc_offset) =
                match next_utc.and_then(|next| format_utc_ms(next, timezone)) {
                    Some((local, offset)) => (Some(local), Some(offset)),
                    None => (None, None),
                };
            let relative_ms = match (next_utc, wall_now_ms) {
                (Some(next), Some(now)) => Some(next.saturating_sub(now)),
                _ => None,
            };
            let last_result = state
                .executions
                .iter()
                .rev()
                .find(|record| {
                    record.trigger.trigger_type == "schedule"
                        && record.trigger.name.as_deref() == Some(config.name.as_str())
                })
                .map(|record| ScheduleLastResultSnapshot {
                    status: if record.status == LogicExecutionStatus::Succeeded {
                        "delivered"
                    } else {
                        "failed"
                    }
                    .to_owned(),
                    execution_id: record.execution_id,
                    time_ms: record.time_ms,
                });
            ScheduleSnapshot {
                name: config.name.clone(),
                enabled: config.enabled,
                status: status.to_owned(),
                rule: config.rule.clone(),
                next_occurrence,
                next_occurrence_utc_ms: next_utc,
                relative_ms,
                utc_offset,
                unavailable_reason,
                last_result,
            }
        })
        .collect()
}

fn snapshot_locked(inner: &Inner, _now: logiksmith_core::MonotonicMs) -> Snapshot {
    let values = ValuesSnapshot {
        endpoints: inner
            .endpoint_values
            .iter()
            .map(|(name, state)| EndpointValueSnapshot {
                name: name.to_string(),
                direction: state.direction.to_string(),
                dpt: DptMessage::from_core(state.dpt),
                observed: state.observed.clone(),
                requested: state.requested.clone(),
            })
            .collect(),
    };
    let blocks = inner
        .block_order
        .iter()
        .filter_map(|id| inner.blocks.get(id).map(|state| (id, state)))
        .map(|(id, state)| {
            let automation = inner.block_automation.get(id);
            BlockSnapshot {
                id: id.clone(),
                active_enabled: state.active_enabled,
                saved_enabled: state.saved_enabled,
                active_revision: state.active_logic_revision,
                saved_revision: state.saved_logic_revision,
                active_logic_revision: state.active_logic_revision,
                saved_logic_revision: state.saved_logic_revision,
                source: if state.source.is_empty() {
                    automation
                        .map(|automation| automation.logic.source.clone())
                        .unwrap_or_default()
                } else {
                    state.source.clone()
                },
                inputs: automation
                    .map(|automation| automation.inputs.clone())
                    .unwrap_or_default(),
                outputs: automation
                    .map(|automation| automation.outputs.clone())
                    .unwrap_or_default(),
                knx_bindings: automation
                    .map(|automation| automation.knx_bindings.clone())
                    .unwrap_or_default(),
                signal_bindings: automation
                    .map(|automation| automation.signal_bindings.clone())
                    .unwrap_or_default(),
                values: ValuesSnapshot {
                    endpoints: inner
                        .block_endpoint_values
                        .iter()
                        .filter(|((block_id, _), _)| block_id == id)
                        .map(|((_, name), state)| EndpointValueSnapshot {
                            name: name.to_string(),
                            direction: state.direction.to_string(),
                            dpt: DptMessage::from_core(state.dpt),
                            observed: state.observed.clone(),
                            requested: state.requested.clone(),
                        })
                        .collect(),
                },
                state: state.state.clone(),
                pending_timers: state.pending_timers.clone(),
                executions: state.executions.iter().rev().cloned().collect(),
                schedules: block_schedule_snapshots(inner, id, state),
                last_result: state.last_result.clone(),
            }
        })
        .collect();
    Snapshot {
        revision: inner.revision,
        connection: ConnectionSnapshot {
            state: inner.connection,
        },
        config: ConfigSnapshot {
            active: inner.automation.clone(),
        },
        automation: inner.automation.clone(),
        active_automation_revision: inner.active_automation_revision,
        saved_automation_revision: inner.saved_automation_revision,
        captured_at_ms: inner.captured_at_ms,
        site_time: inner.site_time.clone(),
        state: inner.state.clone(),
        pending_timers: inner.pending_timers.clone(),
        values,
        write: inner.last_write.clone(),
        logic: LogicStatusSnapshot {
            active_logic_revision: inner.active_logic_revision,
            saved_logic_revision: inner.saved_logic_revision,
            active_structural_revision: inner.active_structural_revision,
            saved_structural_revision: inner.saved_structural_revision,
            restart_required: inner.restart_required,
            state: inner.state.clone(),
            pending_timers: inner.pending_timers.clone(),
            executions: inner.executions.iter().rev().cloned().collect(),
        },
        telegrams: inner.telegrams.iter().cloned().collect(),
        logs: inner.logs.iter().cloned().collect(),
        blocks,
        signals: inner.signals.clone(),
    }
}
