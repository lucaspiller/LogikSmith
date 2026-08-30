use crate::*;

/// Returns the first occurrence strictly after both the current wall-clock
/// sample and any occurrence already delivered for this schedule.
pub(crate) fn next_occurrence_after_not_before(
    rule: &ScheduleRule,
    site: &SiteTimeConfig,
    now_utc: i64,
    last_delivered_utc_ms: Option<i64>,
) -> Option<i64> {
    let baseline = last_delivered_utc_ms
        .map(|last| last.max(now_utc))
        .unwrap_or(now_utc);
    schedule::next_occurrence_after(rule, site, baseline)
}

pub(crate) fn validate_simulation_input(
    endpoint: &Endpoint,
    input: &SimulationInput,
) -> Result<(), SimulationError> {
    if input.valid {
        let value = input
            .value
            .ok_or_else(|| SimulationError::MissingValue(input.endpoint.clone()))?;
        if input.age_ms.is_none() {
            return Err(SimulationError::MissingAge(input.endpoint.clone()));
        }
        validate_simulation_value(endpoint, value)
    } else {
        if input.value.is_some() {
            return Err(SimulationError::UnexpectedValue(input.endpoint.clone()));
        }
        if input.age_ms.is_some() {
            return Err(SimulationError::UnexpectedAge(input.endpoint.clone()));
        }
        Ok(())
    }
}

pub(crate) fn validate_simulation_value(
    endpoint: &Endpoint,
    value: TypedValue,
) -> Result<(), SimulationError> {
    if endpoint.dpt != value.dpt() {
        return Err(SimulationError::DptMismatch {
            endpoint: endpoint.name.clone(),
            expected: endpoint.dpt,
            actual: value.dpt(),
        });
    }
    Ok(())
}

pub(crate) fn input_trigger(
    endpoint: EndpointName,
    value: TypedValue,
    previous: Option<TypedValue>,
) -> InputTrigger {
    InputTrigger {
        endpoint,
        value,
        previous,
        changed: previous.is_some_and(|previous| previous != value),
        rising: matches!(
            (previous.map(TypedValue::value), value.value()),
            (Some(Value::Bool(false)), Value::Bool(true))
        ),
        falling: matches!(
            (previous.map(TypedValue::value), value.value()),
            (Some(Value::Bool(true)), Value::Bool(false))
        ),
    }
}

pub(crate) fn default_site() -> SiteTimeConfig {
    SiteTimeConfig {
        timezone: TimeZoneId::utc(),
        coordinates: None,
    }
}

/// The unavailable time-context sentinel for legacy and simulation paths that
/// have no wall-clock instant.
pub(crate) fn unavailable_time_context() -> TimeContext {
    TimeContext::capture(&default_site(), None)
}
