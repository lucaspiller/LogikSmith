use crate::{
    AutomationDocument, AutomationRuntime, BlockRuntime, FieldError, Revision,
    build_automation_with_capabilities, document_logic_revision,
};
use logiksmith_core::{
    BlockActivation, BlockId, Execution, InputEvent, InputObservation, InputSnapshot, MonotonicMs,
    PendingTimer, Runtime as CoreRuntime, RuntimeProfile, SimulationScenario, SimulationTrigger,
    StateValue, TimerName, TimerSimulationScenario, TypedValue, Value,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value")]
pub enum ValueMessage {
    Bool(bool),
    Percent(u8),
    Temperature(f64),
}
impl ValueMessage {
    pub fn to_core(&self, dpt: logiksmith_core::Dpt, path: &str) -> Result<TypedValue, FieldError> {
        let value = match self {
            Self::Bool(v) => Value::Bool(*v),
            Self::Percent(v) => Value::Percent(*v),
            Self::Temperature(v) => {
                return TypedValue::temperature(*v).map_err(|e| FieldError {
                    path: path.into(),
                    message: e.to_string(),
                });
            }
        };
        TypedValue::new(dpt, value).map_err(|e| FieldError {
            path: path.into(),
            message: e.to_string(),
        })
    }
    pub fn from_core(value: TypedValue) -> Self {
        match value.value() {
            Value::Bool(v) => Self::Bool(v),
            Value::Percent(v) => Self::Percent(v),
            Value::Temperature(v) => Self::Temperature(f64::from(v) / 100.0),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SimulationInput {
    pub endpoint: String,
    pub value: Option<ValueMessage>,
    pub valid: bool,
    pub age_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SimulationRequest {
    pub block_id: String,
    #[serde(default)]
    pub source: Option<String>,
    pub trigger_endpoint: String,
    pub trigger_value: ValueMessage,
    #[serde(default)]
    pub previous: Option<ValueMessage>,
    #[serde(default)]
    pub inputs: Vec<SimulationInput>,
    #[serde(default)]
    pub state: BTreeMap<String, StateMessage>,
    #[serde(default)]
    pub pending_timers: Vec<TimerMessage>,
    #[serde(default)]
    pub now_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TimerSimulationRequest {
    pub block_id: String,
    pub timer: String,
    pub fired_at_ms: u64,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub inputs: Vec<SimulationInput>,
    #[serde(default)]
    pub state: BTreeMap<String, StateMessage>,
    #[serde(default)]
    pub pending_timers: Vec<TimerMessage>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value")]
pub enum StateMessage {
    Bool(bool),
    Integer(i64),
    Number(f64),
    String(String),
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TimerMessage {
    pub name: String,
    pub scheduled_at_ms: u64,
    pub due_at_ms: u64,
    pub logic_revision: Revision,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutionProjection {
    pub id: Option<Revision>,
    pub logic_revision: Revision,
    pub trigger: TriggerProjection,
    pub inputs: Vec<InputProjection>,
    pub state_before: BTreeMap<String, StateMessage>,
    pub state_after: BTreeMap<String, StateMessage>,
    pub pending_timers: Vec<TimerMessage>,
    pub outcome: OutcomeProjection,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum TriggerProjection {
    Input {
        endpoint: String,
        value: ValueMessage,
        previous: Option<ValueMessage>,
        changed: bool,
        rising: bool,
        falling: bool,
    },
    Timer {
        name: String,
        scheduled_at_ms: u64,
        due_at_ms: u64,
        fired_at_ms: u64,
    },
    Schedule {
        name: String,
        scheduled_for_utc_ms: i64,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InputProjection {
    pub endpoint: String,
    pub dpt: String,
    pub value: Option<ValueMessage>,
    pub valid: bool,
    pub age_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OutcomeProjection {
    pub ok: bool,
    pub state: BTreeMap<String, StateMessage>,
    pub outputs: Vec<OutputProjection>,
    pub timers: Vec<TimerEffectProjection>,
    pub persistent: PersistentEffectProjection,
    pub error: Option<SourceErrorProjection>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OutputProjection {
    pub endpoint: String,
    pub value: ValueMessage,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TimerEffectProjection {
    pub name: String,
    pub action: String,
    pub due_at_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PersistentEffectProjection {
    pub set: BTreeMap<String, StateMessage>,
    pub delete: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceErrorProjection {
    pub category: String,
    pub message: String,
    pub line: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BlockProjection {
    pub id: String,
    pub enabled: bool,
    pub health: String,
    pub consecutive_failures: u32,
    pub logic_revision: Revision,
    pub inputs: Vec<InputProjection>,
    pub state: BTreeMap<String, StateMessage>,
    pub persistent_state: BTreeMap<String, StateMessage>,
    pub pending_timers: Vec<TimerMessage>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RuntimeProjection {
    pub document_revision: Revision,
    pub structural_revision: Revision,
    pub logic_revision: Revision,
    pub blocks: Vec<BlockProjection>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ActivationProjection {
    pub document_revision: Revision,
    pub structural_revision: Revision,
    pub blocks: Vec<ActivationBlockProjection>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ActivationBlockProjection {
    pub block_id: String,
    pub block_revision: Revision,
    pub logic_revision: Revision,
    pub enabled: bool,
    pub source_changed: bool,
    pub enabled_changed: bool,
    pub cancelled_timers: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ValidationProjection {
    pub valid: bool,
    pub document_revision: Revision,
    pub structural_revision: Revision,
    pub errors: Vec<FieldError>,
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum OperationError {
    #[error("unknown block {block_id}")]
    UnknownBlock { block_id: String },
    #[error("stale document or block revision")]
    Stale {
        current_document_revision: Revision,
        current_structural_revision: Revision,
        current_block_revision: Option<Revision>,
    },
    #[error("invalid operation")]
    Invalid { errors: Vec<FieldError> },
    #[error("runtime operation failed: {0}")]
    Runtime(String),
}

#[derive(Clone, Debug)]
pub struct ActivationRequest {
    pub block_id: String,
    pub source: Option<String>,
    pub enabled: Option<bool>,
    pub expected_document_revision: Revision,
    pub expected_structural_revision: Revision,
    pub expected_block_revision: Revision,
    pub new_document_revision: Revision,
}

/// The shared stateful runtime adapter. The host owns its instance and decides
/// when to persist a candidate document; this type only mutates live state.
#[derive(Clone, Debug)]
pub struct RuntimeApi {
    pub(crate) runtime: CoreRuntime,
    pub(crate) document: AutomationDocument,
    pub(crate) automation: AutomationRuntime,
    pub(crate) structural_revision: u64,
    pub(crate) document_revision: u64,
    pub(crate) block_revisions: BTreeMap<String, u64>,
    capabilities: crate::Capabilities,
}

impl RuntimeApi {
    pub fn new(automation: AutomationRuntime) -> Result<Self, OperationError> {
        Self::from_document(automation.document, RuntimeProfile::Desktop)
    }
    pub fn from_document(
        document: AutomationDocument,
        profile: RuntimeProfile,
    ) -> Result<Self, OperationError> {
        let caps = if profile == RuntimeProfile::EmbeddedBaseline {
            crate::Capabilities::embedded()
        } else {
            crate::Capabilities::desktop()
        };
        let automation = build_automation_with_capabilities(document.clone(), profile, caps)
            .map_err(|errors| OperationError::Invalid { errors })?;
        let runtime = CoreRuntime::try_new_with_profile(automation.core_config.clone(), profile)
            .map_err(|e| OperationError::Runtime(e.to_string()))?;
        let block_revisions = automation
            .document
            .blocks
            .iter()
            .map(|b| (b.id.clone(), b.revision.max(1)))
            .collect();
        Ok(Self {
            runtime,
            document,
            automation: automation.clone(),
            structural_revision: automation.structural_revision,
            document_revision: 0,
            block_revisions,
            capabilities: caps,
        })
    }
    pub fn capabilities(&self) -> crate::Capabilities {
        self.capabilities
    }
    pub fn runtime(&self) -> &CoreRuntime {
        &self.runtime
    }
    pub fn runtime_mut(&mut self) -> &mut CoreRuntime {
        &mut self.runtime
    }
    pub fn document(&self) -> &AutomationDocument {
        &self.document
    }
    pub fn revisions(&self) -> (Revision, Revision) {
        (
            self.document_revision.into(),
            self.structural_revision.into(),
        )
    }
    pub fn validate_document(
        document: AutomationDocument,
        profile: RuntimeProfile,
    ) -> ValidationProjection {
        match build_automation_with_capabilities(
            document,
            profile,
            if profile == RuntimeProfile::EmbeddedBaseline {
                crate::Capabilities::embedded()
            } else {
                crate::Capabilities::desktop()
            },
        ) {
            Ok(runtime) => ValidationProjection {
                valid: true,
                document_revision: 0.into(),
                structural_revision: runtime.structural_revision.into(),
                errors: Vec::new(),
            },
            Err(errors) => ValidationProjection {
                valid: false,
                document_revision: 0.into(),
                structural_revision: 0.into(),
                errors,
            },
        }
    }
    pub fn validate_source(source: &str, profile: RuntimeProfile) -> ValidationProjection {
        match logiksmith_core::LogicProgram::try_new_with_limits(source, profile.limits()) {
            Ok(_) => ValidationProjection {
                valid: true,
                document_revision: 0.into(),
                structural_revision: 0.into(),
                errors: Vec::new(),
            },
            Err(e) => ValidationProjection {
                valid: false,
                document_revision: 0.into(),
                structural_revision: 0.into(),
                errors: vec![FieldError {
                    path: "source".into(),
                    message: e.to_string(),
                }],
            },
        }
    }
    pub fn deliver_input(
        &mut self,
        block_id: &str,
        endpoint: &str,
        value: ValueMessage,
        now_ms: u64,
    ) -> Result<Option<ExecutionProjection>, OperationError> {
        let endpoint = endpoint.parse().map_err(|_| OperationError::Invalid {
            errors: vec![FieldError {
                path: "endpoint".into(),
                message: "invalid endpoint".into(),
            }],
        })?;
        let (block_id, dpt) = {
            let block = self.block(block_id)?;
            (
                block.id.clone(),
                block.endpoint_dpts.get(&endpoint).copied().ok_or_else(|| {
                    OperationError::Invalid {
                        errors: vec![FieldError {
                            path: "endpoint".into(),
                            message: "unknown endpoint".into(),
                        }],
                    }
                })?,
            )
        };
        let typed = value
            .to_core(dpt, "value")
            .map_err(|e| OperationError::Invalid { errors: vec![e] })?;
        self.runtime
            .process_input(
                &block_id,
                InputEvent::new(endpoint, typed),
                MonotonicMs(now_ms),
            )
            .map(|e| e.map(|e| execution_projection(&e.execution)))
            .map_err(|e| OperationError::Runtime(e.to_string()))
    }
    pub fn observe_input(
        &mut self,
        block_id: &str,
        endpoint: &str,
        value: ValueMessage,
        now_ms: u64,
    ) -> Result<(), OperationError> {
        let ep = endpoint.parse().map_err(|_| OperationError::Invalid {
            errors: vec![FieldError {
                path: "endpoint".into(),
                message: "invalid endpoint".into(),
            }],
        })?;
        let (block_id, dpt) =
            {
                let block = self.block(block_id)?;
                (
                    block.id.clone(),
                    block.endpoint_dpts.get(&ep).copied().ok_or_else(|| {
                        OperationError::Invalid {
                            errors: vec![FieldError {
                                path: "endpoint".into(),
                                message: "unknown endpoint".into(),
                            }],
                        }
                    })?,
                )
            };
        let value = value
            .to_core(dpt, "value")
            .map_err(|e| OperationError::Invalid { errors: vec![e] })?;
        self.runtime
            .observe_input(
                &block_id,
                InputObservation::new(ep, value),
                MonotonicMs(now_ms),
            )
            .map_err(|e| OperationError::Runtime(e.to_string()))
    }
    /// Alias used by hosts which call every ingress path an input process.
    pub fn process_input(
        &mut self,
        block_id: &str,
        endpoint: &str,
        value: ValueMessage,
        now_ms: u64,
    ) -> Result<Option<ExecutionProjection>, OperationError> {
        self.deliver_input(block_id, endpoint, value, now_ms)
    }
    pub fn process_due_timer(
        &mut self,
        now_ms: u64,
    ) -> Result<Option<ExecutionProjection>, OperationError> {
        self.runtime
            .process_next_due_timer(MonotonicMs(now_ms))
            .map(|e| e.map(|e| execution_projection(&e.execution)))
            .map_err(|e| OperationError::Runtime(e.to_string()))
    }
    pub fn activate(
        &mut self,
        request: ActivationRequest,
    ) -> Result<ActivationProjection, OperationError> {
        self.check_cas(
            &request.block_id,
            request.expected_document_revision,
            request.expected_structural_revision,
            request.expected_block_revision,
        )?;
        let id: BlockId = request
            .block_id
            .parse()
            .map_err(|_| OperationError::UnknownBlock {
                block_id: request.block_id.clone(),
            })?;
        let result = self
            .runtime
            .activate(logiksmith_core::RuntimeActivation::single(
                BlockActivation::new(id.clone(), request.source.clone(), request.enabled),
            ))
            .map_err(|e| OperationError::Runtime(e.to_string()))?;
        let block = self
            .document
            .blocks
            .iter_mut()
            .find(|b| b.id == request.block_id)
            .ok_or_else(|| OperationError::UnknownBlock {
                block_id: request.block_id.clone(),
            })?;
        if let Some(source) = request.source {
            block.source = source;
        }
        if let Some(enabled) = request.enabled {
            block.enabled = enabled;
        }
        let new_revision = request
            .new_document_revision
            .0
            .max(self.document_revision.saturating_add(1));
        block.revision = block.revision.saturating_add(1).max(1);
        self.document_revision = new_revision;
        self.block_revisions
            .insert(block.id.clone(), block.revision);
        let structural = self.structural_revision;
        let block_revision = block.revision;
        let blocks = result
            .blocks
            .into_iter()
            .map(|b| ActivationBlockProjection {
                block_id: b.block_id.to_string(),
                block_revision: block_revision.into(),
                logic_revision: b.logic_revision.into(),
                enabled: b.enabled,
                source_changed: b.source_changed,
                enabled_changed: b.enabled_changed,
                cancelled_timers: b
                    .cancelled_timers
                    .into_iter()
                    .map(|t| t.to_string())
                    .collect(),
            })
            .collect();
        Ok(ActivationProjection {
            document_revision: new_revision.into(),
            structural_revision: structural.into(),
            blocks,
        })
    }
    pub fn activate_source(
        &mut self,
        request: ActivationRequest,
    ) -> Result<ActivationProjection, OperationError> {
        self.activate(request)
    }
    pub fn set_enabled(
        &mut self,
        block_id: &str,
        enabled: bool,
        expected_document_revision: Revision,
        expected_structural_revision: Revision,
        expected_block_revision: Revision,
        new_document_revision: Revision,
    ) -> Result<ActivationProjection, OperationError> {
        self.activate(ActivationRequest {
            block_id: block_id.into(),
            source: None,
            enabled: Some(enabled),
            expected_document_revision,
            expected_structural_revision,
            expected_block_revision,
            new_document_revision,
        })
    }
    pub fn resume(
        &mut self,
        block_id: &str,
        expected_document_revision: Revision,
        expected_structural_revision: Revision,
        expected_block_revision: Revision,
    ) -> Result<(), OperationError> {
        self.check_cas(
            block_id,
            expected_document_revision,
            expected_structural_revision,
            expected_block_revision,
        )?;
        let id = block_id.parse().map_err(|_| OperationError::UnknownBlock {
            block_id: block_id.into(),
        })?;
        self.runtime
            .resume_block(&id)
            .map_err(|e| OperationError::Runtime(e.to_string()))
    }
    pub fn resume_block(
        &mut self,
        block_id: &str,
        expected_document_revision: Revision,
        expected_structural_revision: Revision,
        expected_block_revision: Revision,
    ) -> Result<(), OperationError> {
        self.resume(
            block_id,
            expected_document_revision,
            expected_structural_revision,
            expected_block_revision,
        )
    }
    pub fn simulate_input(
        &self,
        request: SimulationRequest,
    ) -> Result<ExecutionProjection, OperationError> {
        let block = self.block(&request.block_id)?;
        let scenario = input_scenario(block, &request)?;
        let state = state_map(&request.state)?;
        let timers = pending_timers(&request.pending_timers)?;
        let result = if let Some(source) = request.source {
            self.runtime.simulate_input_with_source(
                &block.id,
                source,
                scenario,
                state,
                timers,
                MonotonicMs(request.now_ms),
            )
        } else {
            self.runtime.simulate_input_with_state(
                &block.id,
                scenario,
                state,
                timers,
                MonotonicMs(request.now_ms),
            )
        };
        result
            .map(|b| execution_projection(&b.execution))
            .map_err(|e| OperationError::Runtime(e.to_string()))
    }
    pub fn simulate_timer(
        &self,
        request: TimerSimulationRequest,
    ) -> Result<ExecutionProjection, OperationError> {
        let block = self.block(&request.block_id)?;
        let timer: TimerName =
            request
                .timer
                .parse::<TimerName>()
                .map_err(|e| OperationError::Invalid {
                    errors: vec![FieldError {
                        path: "timer".into(),
                        message: e.to_string(),
                    }],
                })?;
        let scenario = TimerSimulationScenario {
            timer,
            fired_at: MonotonicMs(request.fired_at_ms),
            inputs: simulation_inputs(block, &request.inputs)?,
            state: state_map(&request.state)?,
            pending_timers: pending_timers(&request.pending_timers)?,
        };
        let result = if let Some(source) = request.source {
            self.runtime
                .simulate_timer_with_source(&block.id, source, scenario)
        } else {
            self.runtime.simulate_timer(&block.id, scenario)
        };
        result
            .map(|b| execution_projection(&b.execution))
            .map_err(|e| OperationError::Runtime(e.to_string()))
    }
    pub fn projection(&self, now_ms: u64) -> RuntimeProjection {
        let snap = self.runtime.snapshot_at(MonotonicMs(now_ms));
        RuntimeProjection {
            document_revision: self.document_revision.into(),
            structural_revision: self.structural_revision.into(),
            logic_revision: document_logic_revision(&self.document).into(),
            blocks: snap
                .blocks
                .into_iter()
                .map(|b| BlockProjection {
                    id: b.id.to_string(),
                    enabled: b.enabled,
                    health: b.health.as_str().into(),
                    consecutive_failures: b.consecutive_failures,
                    logic_revision: b.logic_revision.into(),
                    inputs: b.inputs.into_iter().map(input_projection).collect(),
                    state: state_projection(&b.state),
                    persistent_state: state_projection(&b.persistent_state),
                    pending_timers: b.pending_timers.into_iter().map(timer_projection).collect(),
                })
                .collect(),
        }
    }
    fn block(&self, id: &str) -> Result<&BlockRuntime, OperationError> {
        let bid = id
            .parse::<BlockId>()
            .map_err(|_| OperationError::UnknownBlock {
                block_id: id.into(),
            })?;
        self.automation
            .block(&bid)
            .ok_or_else(|| OperationError::UnknownBlock {
                block_id: id.into(),
            })
    }
    fn check_cas(
        &self,
        id: &str,
        doc: Revision,
        structural: Revision,
        block: Revision,
    ) -> Result<(), OperationError> {
        let current = self.block_revisions.get(id).copied();
        if doc.0 != self.document_revision
            || structural.0 != self.structural_revision
            || current != Some(block.0)
        {
            return Err(OperationError::Stale {
                current_document_revision: self.document_revision.into(),
                current_structural_revision: self.structural_revision.into(),
                current_block_revision: current.map(Into::into),
            });
        }
        Ok(())
    }
}

fn state_map(
    values: &BTreeMap<String, StateMessage>,
) -> Result<logiksmith_core::TransientState, OperationError> {
    values
        .iter()
        .map(|(k, v)| {
            let value = match v {
                StateMessage::Bool(x) => StateValue::Bool(*x),
                StateMessage::Integer(x) => StateValue::Integer(*x),
                StateMessage::Number(x) if x.is_finite() => StateValue::Number(*x),
                StateMessage::Number(_) => {
                    return Err(OperationError::Invalid {
                        errors: vec![FieldError {
                            path: format!("state.{k}"),
                            message: "must be finite".into(),
                        }],
                    });
                }
                StateMessage::String(x) => StateValue::String(x.clone()),
            };
            Ok((k.clone(), value))
        })
        .collect()
}
fn pending_timers(values: &[TimerMessage]) -> Result<Vec<PendingTimer>, OperationError> {
    values
        .iter()
        .map(|v| {
            Ok(PendingTimer {
                name: v
                    .name
                    .parse::<TimerName>()
                    .map_err(|e| OperationError::Invalid {
                        errors: vec![FieldError {
                            path: "pending_timers.name".into(),
                            message: e.to_string(),
                        }],
                    })?,
                scheduled_at: MonotonicMs(v.scheduled_at_ms),
                due_at: MonotonicMs(v.due_at_ms),
                scheduled_logic_revision: v.logic_revision.0,
            })
        })
        .collect()
}
fn simulation_inputs(
    block: &BlockRuntime,
    values: &[SimulationInput],
) -> Result<Vec<logiksmith_core::SimulationInput>, OperationError> {
    values
        .iter()
        .map(|input| {
            let ep = input
                .endpoint
                .parse::<logiksmith_core::EndpointName>()
                .map_err(|_| OperationError::Invalid {
                    errors: vec![FieldError {
                        path: "inputs.endpoint".into(),
                        message: "invalid endpoint".into(),
                    }],
                })?;
            let dpt =
                block
                    .endpoint_dpts
                    .get(&ep)
                    .copied()
                    .ok_or_else(|| OperationError::Invalid {
                        errors: vec![FieldError {
                            path: "inputs.endpoint".into(),
                            message: "unknown endpoint".into(),
                        }],
                    })?;
            let value = match (&input.value, input.valid) {
                (Some(v), true) => Some(
                    v.to_core(dpt, "inputs.value")
                        .map_err(|e| OperationError::Invalid { errors: vec![e] })?,
                ),
                (_, false) => None,
                (None, true) => {
                    return Err(OperationError::Invalid {
                        errors: vec![FieldError {
                            path: "inputs.value".into(),
                            message: "required for a valid input".into(),
                        }],
                    });
                }
            };
            Ok(logiksmith_core::SimulationInput {
                endpoint: ep,
                value,
                valid: input.valid,
                age_ms: input.age_ms,
            })
        })
        .collect()
}
fn input_scenario(
    block: &BlockRuntime,
    request: &SimulationRequest,
) -> Result<SimulationScenario, OperationError> {
    let ep = request
        .trigger_endpoint
        .parse()
        .map_err(|_| OperationError::Invalid {
            errors: vec![FieldError {
                path: "trigger_endpoint".into(),
                message: "invalid endpoint".into(),
            }],
        })?;
    let dpt = block
        .endpoint_dpts
        .get(&ep)
        .copied()
        .ok_or_else(|| OperationError::Invalid {
            errors: vec![FieldError {
                path: "trigger_endpoint".into(),
                message: "unknown endpoint".into(),
            }],
        })?;
    let value = request
        .trigger_value
        .to_core(dpt, "trigger_value")
        .map_err(|e| OperationError::Invalid { errors: vec![e] })?;
    let previous = request
        .previous
        .as_ref()
        .map(|v| v.to_core(dpt, "previous"))
        .transpose()
        .map_err(|e| OperationError::Invalid { errors: vec![e] })?;
    Ok(SimulationScenario {
        trigger: SimulationTrigger {
            endpoint: ep,
            value,
            previous,
        },
        inputs: simulation_inputs(block, &request.inputs)?,
    })
}
fn state_projection(state: &logiksmith_core::TransientState) -> BTreeMap<String, StateMessage> {
    state
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                match v {
                    StateValue::Bool(x) => StateMessage::Bool(*x),
                    StateValue::Integer(x) => StateMessage::Integer(*x),
                    StateValue::Number(x) => StateMessage::Number(*x),
                    StateValue::String(x) => StateMessage::String(x.clone()),
                },
            )
        })
        .collect()
}
fn persistent_projection(
    patch: &logiksmith_core::PersistentPatch,
) -> PersistentEffectProjection {
    PersistentEffectProjection {
        set: state_projection(&patch.set),
        delete: patch.delete.iter().cloned().collect(),
    }
}
fn input_projection(input: InputSnapshot) -> InputProjection {
    InputProjection {
        endpoint: input.endpoint.to_string(),
        dpt: input.dpt.to_string(),
        value: input.value.map(ValueMessage::from_core),
        valid: input.valid,
        age_ms: input.age_ms,
    }
}
fn timer_projection(timer: PendingTimer) -> TimerMessage {
    TimerMessage {
        name: timer.name.to_string(),
        scheduled_at_ms: timer.scheduled_at.0,
        due_at_ms: timer.due_at.0,
        logic_revision: timer.scheduled_logic_revision.into(),
    }
}
fn execution_projection(execution: &Execution) -> ExecutionProjection {
    let trigger = match &execution.trigger {
        logiksmith_core::Trigger::Input(t) => TriggerProjection::Input {
            endpoint: t.endpoint.to_string(),
            value: ValueMessage::from_core(t.value),
            previous: t.previous.map(ValueMessage::from_core),
            changed: t.changed,
            rising: t.rising,
            falling: t.falling,
        },
        logiksmith_core::Trigger::Timer(t) => TriggerProjection::Timer {
            name: t.name.to_string(),
            scheduled_at_ms: t.scheduled_at.0,
            due_at_ms: t.due_at.0,
            fired_at_ms: t.fired_at.0,
        },
        logiksmith_core::Trigger::Schedule(t) => TriggerProjection::Schedule {
            name: t.name.to_string(),
            scheduled_for_utc_ms: t.scheduled_for_utc_ms,
        },
    };
    let outcome = match &execution.outcome {
        Ok(t) => OutcomeProjection {
            ok: true,
            state: state_projection(&t.state),
            outputs: t
                .outputs
                .iter()
                .map(|o| OutputProjection {
                    endpoint: o.endpoint.to_string(),
                    value: ValueMessage::from_core(o.value),
                })
                .collect(),
            timers: t
                .timers
                .iter()
                .map(|t| TimerEffectProjection {
                    name: t.name.to_string(),
                    action: format!("{:?}", t.action),
                    due_at_ms: match &t.action {
                        logiksmith_core::TimerAction::Scheduled { due_at, .. }
                        | logiksmith_core::TimerAction::Replaced { due_at, .. } => Some(due_at.0),
                        _ => None,
                    },
                })
                .collect(),
            persistent: persistent_projection(&t.persistent),
            error: None,
        },
        Err(e) => OutcomeProjection {
            ok: false,
            state: BTreeMap::new(),
            outputs: Vec::new(),
            timers: Vec::new(),
            persistent: PersistentEffectProjection {
                set: BTreeMap::new(),
                delete: Vec::new(),
            },
            error: Some(SourceErrorProjection {
                category: e.category().into(),
                message: e.message().into(),
                line: e.line(),
            }),
        },
    };
    ExecutionProjection {
        id: execution.id.map(Into::into),
        logic_revision: execution.logic_revision.into(),
        trigger,
        inputs: execution
            .inputs
            .clone()
            .into_iter()
            .map(input_projection)
            .collect(),
        state_before: state_projection(&execution.state_before),
        state_after: state_projection(&execution.state_after),
        pending_timers: execution
            .pending_timers
            .clone()
            .into_iter()
            .map(timer_projection)
            .collect(),
        outcome,
    }
}
