use crate::lua::validate_logic_source;
use crate::program::revision_for;
use crate::*;

/// A source-backed portable endpoint engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineConfig {
    pub endpoints: Vec<Endpoint>,
    pub logic: LogicProgram,
}

impl EngineConfig {
    pub fn new(endpoints: Vec<Endpoint>, source: impl Into<String>) -> Self {
        Self {
            endpoints,
            logic: LogicProgram::new(source),
        }
    }

    pub fn with_program(endpoints: Vec<Endpoint>, logic: LogicProgram) -> Self {
        Self { endpoints, logic }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        for (index, endpoint) in self.endpoints.iter().enumerate() {
            if self
                .endpoints
                .iter()
                .take(index)
                .any(|other| other.name == endpoint.name)
            {
                return Err(ConfigError::DuplicateEndpoint(endpoint.name.clone()));
            }
            if !endpoint.dpt.is_supported() {
                return Err(ConfigError::UnsupportedEndpointDpt {
                    endpoint: endpoint.name.clone(),
                    dpt: endpoint.dpt,
                });
            }
        }
        let expected_revision = revision_for(&self.logic.source);
        if self.logic.revision != expected_revision {
            return Err(ConfigError::InvalidLogic(LogicError::Load {
                message: "logic source revision does not match its source bytes".to_owned(),
                line: None,
            }));
        }
        validate_logic_source(&self.logic.source).map_err(ConfigError::InvalidLogic)
    }

    pub fn logic_source(&self) -> &str {
        &self.logic.source
    }
}

/// Read-only engine state useful for diagnostics and snapshot endpoints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineSnapshot {
    pub logic_revision: LogicRevision,
    /// Known values in configured input declaration order. Unknown values are
    /// absent, rather than being represented as `false` or `0`.
    pub known_inputs: Vec<(EndpointName, TypedValue)>,
    pub state: TransientState,
    pub pending_timers: Vec<PendingTimer>,
}
