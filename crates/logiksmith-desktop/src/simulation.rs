use crate::configuration::endpoint_name;
use crate::protocol::ProtocolError;
use crate::*;
use logiksmith_core::{
    Dpt, EndpointName, MonotonicMs, PendingTimer, SimulationError, SimulationInput,
    SimulationScenario, SimulationTrigger, StateValue, TimerName, TimerSimulationScenario,
    TypedValue, Value,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DptMessage {
    pub major: u16,
    pub subtype: u16,
}

impl DptMessage {
    pub(crate) fn from_core(dpt: Dpt) -> Self {
        Self {
            major: dpt.major,
            subtype: dpt.subtype,
        }
    }
    pub(crate) fn core(&self, field_name: &'static str) -> Result<Dpt, ProtocolError> {
        let dpt = Dpt::new(self.major, self.subtype)
            .map_err(|error| ProtocolError::Field(field_name, error.to_string()))?;
        if !dpt.is_supported() {
            return Err(ProtocolError::Field(
                field_name,
                "must be 1.001 or 5.001".to_owned(),
            ));
        }
        Ok(dpt)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoolValueMessage {
    pub kind: String,
    pub value: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PercentValueMessage {
    pub kind: String,
    pub value: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ValueMessage {
    Bool(BoolValueMessage),
    Percent(PercentValueMessage),
}

impl ValueMessage {
    pub(crate) fn from_core(value: TypedValue) -> Self {
        match value.value {
            Value::Bool(value) => Self::Bool(BoolValueMessage {
                kind: "bool".to_owned(),
                value,
            }),
            Value::Percent(value) => Self::Percent(PercentValueMessage {
                kind: "percent".to_owned(),
                value,
            }),
        }
    }
    pub(crate) fn core(
        &self,
        dpt: Dpt,
        field_name: &'static str,
    ) -> Result<TypedValue, ProtocolError> {
        let value = match self {
            Self::Bool(value) if value.kind == "bool" => Value::Bool(value.value),
            Self::Percent(value) if value.kind == "percent" => Value::Percent(value.value),
            Self::Bool(_) => {
                return Err(ProtocolError::Field(
                    "value.kind",
                    "must be 'bool'".to_owned(),
                ));
            }
            Self::Percent(_) => {
                return Err(ProtocolError::Field(
                    "value.kind",
                    "must be 'percent'".to_owned(),
                ));
            }
        };
        TypedValue::new(dpt, value)
            .map_err(|error| ProtocolError::Field(field_name, error.to_string()))
    }
}

fn simulation_value(value: &ValueMessage, dpt: Dpt, path: &str) -> Result<TypedValue, FieldError> {
    value.core(dpt, "value").map_err(|error| FieldError {
        path: path.to_owned(),
        message: error.to_string(),
    })
}

fn simulation_state_value(value: &StateValuePayload, path: &str) -> Result<StateValue, FieldError> {
    let invalid = || FieldError {
        path: path.to_owned(),
        message: "must be a tagged bool, integer, number, or string".to_owned(),
    };
    match value.kind.as_str() {
        "bool" => value
            .value
            .as_bool()
            .map(StateValue::Bool)
            .ok_or_else(invalid),
        "integer" => value
            .value
            .as_i64()
            .map(StateValue::Integer)
            .ok_or_else(invalid),
        "number" => value
            .value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(StateValue::Number)
            .ok_or_else(invalid),
        "string" => value
            .value
            .as_str()
            .map(|value| StateValue::String(value.to_owned()))
            .ok_or_else(invalid),
        _ => Err(invalid()),
    }
}

pub(crate) fn simulation_state(
    payload: &SimulationPayload,
    fallback: Option<&logiksmith_core::TransientState>,
) -> Result<logiksmith_core::TransientState, Vec<FieldError>> {
    let mut errors = Vec::new();
    let Some(source) = payload.state.as_ref() else {
        return Ok(fallback.cloned().unwrap_or_default());
    };
    let state = source
        .iter()
        .filter_map(
            |(key, value)| match simulation_state_value(value, &format!("state.{key}")) {
                Ok(value) => Some((key.clone(), value)),
                Err(error) => {
                    errors.push(error);
                    None
                }
            },
        )
        .collect();
    if errors.is_empty() {
        Ok(state)
    } else {
        Err(errors)
    }
}

pub(crate) fn simulation_pending_timers(
    payload: &SimulationPayload,
    fallback: Option<&[PendingTimer]>,
    active_document_revision: u64,
    active_core_revision: u64,
) -> Result<Vec<PendingTimer>, Vec<FieldError>> {
    let Some(source) = payload.pending_timers.as_ref() else {
        return Ok(fallback.map(ToOwned::to_owned).unwrap_or_default());
    };
    let mut errors = Vec::new();
    let timers = source
        .iter()
        .enumerate()
        .filter_map(|(index, timer)| {
            let path = format!("pending_timers[{index}]");
            let name = match timer.name.parse::<TimerName>() {
                Ok(name) => Some(name),
                Err(error) => {
                    errors.push(FieldError {
                        path: format!("{path}.name"),
                        message: error.to_string(),
                    });
                    None
                }
            }?;
            if timer.logic_revision != active_document_revision {
                errors.push(FieldError {
                    path: format!("{path}.logic_revision"),
                    message: "must use the active logic revision".to_owned(),
                });
                return None;
            }
            Some(PendingTimer {
                name,
                scheduled_at: MonotonicMs(timer.scheduled_at_ms),
                due_at: MonotonicMs(timer.due_at_ms),
                scheduled_logic_revision: active_core_revision,
            })
        })
        .collect();
    if errors.is_empty() {
        Ok(timers)
    } else {
        Err(errors)
    }
}

/// Converts the browser wire representation into the core-owned scenario.
/// Endpoint and value checks that depend on the active configuration are kept
/// here, before the immutable core operation is called.
pub(crate) fn simulation_scenario(
    payload: SimulationPayload,
    block: &BlockRuntime,
) -> Result<SimulationScenario, Vec<FieldError>> {
    if payload
        .trigger
        .trigger_type
        .as_deref()
        .is_some_and(|kind| kind != "input")
    {
        return Err(vec![FieldError {
            path: "trigger.type".to_owned(),
            message: "must be 'input' for an input simulation".to_owned(),
        }]);
    }
    let mut errors = Vec::new();
    let trigger_endpoint = match payload.trigger.endpoint.as_deref() {
        Some(endpoint) => match endpoint_name("trigger.endpoint", endpoint) {
            Ok(endpoint) => Some(endpoint),
            Err(error) => {
                errors.push(error);
                None
            }
        },
        None => {
            errors.push(FieldError {
                path: "trigger.endpoint".to_owned(),
                message: "is required for an input simulation".to_owned(),
            });
            None
        }
    };
    let trigger_dpt = trigger_endpoint
        .as_ref()
        .and_then(|endpoint| block.endpoint_dpts.get(endpoint).copied());
    if trigger_endpoint.is_some() && trigger_dpt.is_none() {
        errors.push(FieldError {
            path: "trigger.endpoint".to_owned(),
            message: "must reference an existing input endpoint".to_owned(),
        });
    }
    let trigger_value = trigger_dpt.and_then(|dpt| match payload.trigger.value.as_ref() {
        Some(value) => simulation_value(value, dpt, "trigger.value")
            .map_err(|error| errors.push(error))
            .ok(),
        None => {
            errors.push(FieldError {
                path: "trigger.value".to_owned(),
                message: "is required for an input simulation".to_owned(),
            });
            None
        }
    });
    let previous = match (trigger_dpt, payload.trigger.previous.as_ref()) {
        (Some(dpt), Some(value)) => simulation_value(value, dpt, "trigger.previous")
            .map_err(|error| errors.push(error))
            .ok(),
        (_, None) => None,
        (None, Some(_)) => None,
    };

    let inputs = payload
        .inputs
        .into_iter()
        .enumerate()
        .filter_map(|(index, input)| {
            let path = format!("inputs[{index}]");
            let endpoint = match endpoint_name(&format!("{path}.endpoint"), &input.endpoint) {
                Ok(endpoint) => Some(endpoint),
                Err(error) => {
                    errors.push(error);
                    None
                }
            };
            let dpt = endpoint
                .as_ref()
                .and_then(|endpoint| block.endpoint_dpts.get(endpoint).copied());
            if endpoint.is_some() && dpt.is_none() {
                errors.push(FieldError {
                    path: format!("{path}.endpoint"),
                    message: "must reference an existing input endpoint".to_owned(),
                });
            }
            let value = match (dpt, input.value.as_ref()) {
                (Some(dpt), Some(value)) => simulation_value(value, dpt, &format!("{path}.value"))
                    .map_err(|error| errors.push(error))
                    .ok(),
                (_, None) => None,
                (None, Some(_)) => None,
            };
            let endpoint = endpoint?;
            Some(SimulationInput {
                endpoint,
                value,
                valid: input.valid,
                age_ms: input.age_ms,
            })
        })
        .collect::<Vec<_>>();

    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(SimulationScenario {
        trigger: SimulationTrigger {
            endpoint: trigger_endpoint.expect("validated trigger endpoint"),
            value: trigger_value.expect("validated trigger value"),
            previous,
        },
        inputs,
    })
}

pub(crate) fn simulation_timer_scenario(
    payload: &SimulationPayload,
    block: &BlockRuntime,
    active_document_revision: u64,
    active_core_revision: u64,
    fallback_state: Option<&logiksmith_core::TransientState>,
) -> Result<TimerSimulationScenario, Vec<FieldError>> {
    let mut errors = Vec::new();
    if payload.trigger.trigger_type.as_deref() != Some("timer") {
        errors.push(FieldError {
            path: "trigger.type".to_owned(),
            message: "must be 'timer' for a timer simulation".to_owned(),
        });
    }
    let timer = payload
        .trigger
        .name
        .as_deref()
        .ok_or_else(|| FieldError {
            path: "trigger.name".to_owned(),
            message: "is required for a timer simulation".to_owned(),
        })
        .and_then(|value| {
            value.parse::<TimerName>().map_err(|error| FieldError {
                path: "trigger.name".to_owned(),
                message: error.to_string(),
            })
        });
    let fired_at = payload.trigger.fired_at_ms.ok_or_else(|| FieldError {
        path: "trigger.fired_at_ms".to_owned(),
        message: "is required for a timer simulation".to_owned(),
    });
    let inputs = simulation_input_values(payload, block, &mut errors);
    let state = match simulation_state(payload, fallback_state) {
        Ok(state) => state,
        Err(mut state_errors) => {
            errors.append(&mut state_errors);
            Default::default()
        }
    };
    let pending_timers = match simulation_pending_timers(
        payload,
        None,
        active_document_revision,
        active_core_revision,
    ) {
        Ok(timers) => timers,
        Err(mut timer_errors) => {
            errors.append(&mut timer_errors);
            Vec::new()
        }
    };
    if let Ok(timer) = &timer
        && !pending_timers
            .iter()
            .any(|candidate| candidate.name == *timer)
    {
        errors.push(FieldError {
            path: "trigger.name".to_owned(),
            message: "must select a supplied pending timer".to_owned(),
        });
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(TimerSimulationScenario {
        timer: timer.expect("validated timer"),
        fired_at: MonotonicMs(fired_at.expect("validated fired_at")),
        inputs,
        state,
        pending_timers,
    })
}

fn simulation_input_values(
    payload: &SimulationPayload,
    block: &BlockRuntime,
    errors: &mut Vec<FieldError>,
) -> Vec<SimulationInput> {
    payload
        .inputs
        .iter()
        .enumerate()
        .filter_map(|(index, input)| {
            let path = format!("inputs[{index}]");
            let endpoint = match endpoint_name(&format!("{path}.endpoint"), &input.endpoint) {
                Ok(endpoint) => Some(endpoint),
                Err(error) => {
                    errors.push(error);
                    None
                }
            }?;
            let dpt = block.endpoint_dpts.get(&endpoint).copied();
            if dpt.is_none() {
                errors.push(FieldError {
                    path: format!("{path}.endpoint"),
                    message: "must reference an existing input endpoint".to_owned(),
                });
            }
            let value = match (dpt, input.value.as_ref()) {
                (Some(dpt), Some(value)) => simulation_value(value, dpt, &format!("{path}.value"))
                    .map_err(|error| errors.push(error))
                    .ok(),
                (_, None) => None,
                (None, Some(_)) => None,
            };
            Some(SimulationInput {
                endpoint,
                value,
                valid: input.valid,
                age_ms: input.age_ms,
            })
        })
        .collect()
}

pub(crate) fn simulation_error_fields(
    error: &SimulationError,
    payload: &SimulationPayload,
) -> Vec<FieldError> {
    let endpoint_path = |endpoint: &EndpointName| {
        if payload.trigger.endpoint.as_deref() == Some(endpoint.as_str()) {
            "trigger.endpoint".to_owned()
        } else {
            payload
                .inputs
                .iter()
                .enumerate()
                .find(|(_, input)| input.endpoint == endpoint.as_str())
                .map(|(index, _)| format!("inputs[{index}].endpoint"))
                .unwrap_or_else(|| "inputs".to_owned())
        }
    };
    let input_field = |endpoint: &EndpointName, field_name: &str| {
        payload
            .inputs
            .iter()
            .enumerate()
            .find(|(_, input)| input.endpoint == endpoint.as_str())
            .map(|(index, _)| format!("inputs[{index}].{field_name}"))
            .unwrap_or_else(|| "inputs".to_owned())
    };
    let path = match error {
        SimulationError::UnknownEndpoint(endpoint)
        | SimulationError::EndpointNotInput { endpoint, .. } => endpoint_path(endpoint),
        SimulationError::DuplicateInput(endpoint) | SimulationError::MissingInput(endpoint) => {
            input_field(endpoint, "endpoint")
        }
        SimulationError::DptMismatch { endpoint, .. } => {
            if payload.trigger.endpoint.as_deref() == Some(endpoint.as_str()) {
                "trigger.value".to_owned()
            } else {
                input_field(endpoint, "value")
            }
        }
        SimulationError::InvalidValue(_) => "inputs".to_owned(),
        SimulationError::TriggerValueMismatch { .. } => "trigger.value".to_owned(),
        SimulationError::MissingValue(endpoint) | SimulationError::UnexpectedValue(endpoint) => {
            input_field(endpoint, "value")
        }
        SimulationError::MissingAge(endpoint) | SimulationError::UnexpectedAge(endpoint) => {
            input_field(endpoint, "age_ms")
        }
        SimulationError::TriggerAgeMismatch { endpoint, .. } => input_field(endpoint, "age_ms"),
        SimulationError::UnknownTimer(_) | SimulationError::DuplicateTimer(_) => {
            "trigger.name".to_owned()
        }
        SimulationError::TimerRevisionMismatch { .. } => "pending_timers".to_owned(),
        SimulationError::InvalidState(_) => "state".to_owned(),
        SimulationError::TimeWentBackwards { .. } => "trigger.fired_at_ms".to_owned(),
    };
    vec![FieldError {
        path,
        message: error.to_string(),
    }]
}
