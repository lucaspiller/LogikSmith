//! Compile-time capabilities exposed by the desktop host.
//!
//! Capabilities describe code compiled into this binary. They are deliberately
//! immutable and carry no revision: a process restart (or a new image) is what
//! changes them. Runtime profile and configuration values must not be used to
//! pretend that an omitted feature is available.

use serde::Serialize;

/// The stable JSON contract for the compiled feature set.
///
/// `timezones` and `astronomy` are owned by `logiksmith-core` and forwarded by
/// this crate's Cargo features. The desktop-only network inputs are selected
/// by this crate's Cargo features as well.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CompiledCapabilities {
    pub timezones: bool,
    pub astronomy: bool,
    pub http_inputs: bool,
    pub webhook_inputs: bool,
}

/// Capabilities compiled into this binary.
pub const COMPILED_CAPABILITIES: CompiledCapabilities = CompiledCapabilities {
    timezones: logiksmith_core::compiled_features().timezones,
    astronomy: logiksmith_core::compiled_features().astronomy,
    http_inputs: logiksmith_core::compiled_features().http_inputs,
    webhook_inputs: logiksmith_core::compiled_features().webhook_inputs,
};

/// Returns the immutable capability record used by diagnostics and health
/// responses.
pub const fn compiled_capabilities() -> CompiledCapabilities {
    COMPILED_CAPABILITIES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_json_shape_is_stable() {
        let json = serde_json::to_value(compiled_capabilities()).expect("capabilities serialize");
        assert_eq!(
            json,
            serde_json::json!({
                "timezones": cfg!(feature = "timezones"),
                "astronomy": cfg!(feature = "astronomy"),
                "http_inputs": cfg!(feature = "http-inputs"),
                "webhook_inputs": cfg!(feature = "webhook-inputs")
            })
        );
    }
}
