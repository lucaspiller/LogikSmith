use logiksmith_core::{Engine, InputEvent, MonotonicMs, TypedValue};
use logiksmith_desktop::{
    AutomationDocument, AutomationEndpoint, KnxBinding, LogicDocument, build_automation,
    diagnostics::DiagnosticStore,
};
use serde_json::Value;
use std::path::PathBuf;

fn make_runtime(source: &str) -> logiksmith_desktop::AutomationRuntime {
    build_automation(AutomationDocument {
        inputs: vec![
            AutomationEndpoint {
                name: "wall_switch".to_owned(),
                dpt: "1.001".to_owned(),
            },
            AutomationEndpoint {
                name: "enabled".to_owned(),
                dpt: "1.001".to_owned(),
            },
        ],
        outputs: vec![AutomationEndpoint {
            name: "test_light".to_owned(),
            dpt: "1.001".to_owned(),
        }],
        knx_bindings: vec![
            KnxBinding {
                endpoint: "wall_switch".to_owned(),
                group_address: "2/2/52".to_owned(),
            },
            KnxBinding {
                endpoint: "enabled".to_owned(),
                group_address: "2/2/53".to_owned(),
            },
            KnxBinding {
                endpoint: "test_light".to_owned(),
                group_address: "2/3/52".to_owned(),
            },
        ],
        logic: LogicDocument {
            source: source.to_owned(),
        },
    })
    .expect("valid automation")
}

fn bool_event(name: &str, value: bool) -> InputEvent {
    InputEvent::new(name.parse().unwrap(), TypedValue::bool(value))
}

fn store(runtime: &logiksmith_desktop::AutomationRuntime) -> DiagnosticStore {
    DiagnosticStore::new(runtime, PathBuf::from("automation.toml"), 1)
}

#[test]
fn records_zero_effect_and_failure_with_frozen_snapshots() {
    let runtime = make_runtime("function handle(event, input, meta) return nil end");
    let mut engine = Engine::new(runtime.engine_config.clone());
    let store = store(&runtime);

    let success = engine
        .process_input(bool_event("wall_switch", true), MonotonicMs(10))
        .unwrap();
    assert!(success.outcome.as_ref().unwrap().is_empty());
    store.record_execution_at(&success, MonotonicMs(10), 17, &runtime);

    let failure_runtime = make_runtime("function handle(event, input, meta) error('boom') end");
    let mut failure_engine = Engine::new(failure_runtime.engine_config.clone());
    let failure = failure_engine
        .process_input(bool_event("wall_switch", true), MonotonicMs(10))
        .unwrap();
    assert!(failure.outcome.is_err());
    store.record_execution_at(&failure, MonotonicMs(11), 23, &failure_runtime);

    let snapshot = store.snapshot_at(MonotonicMs(11));
    assert_eq!(snapshot.logic.executions.len(), 2);
    assert_eq!(snapshot.logic.executions[0].execution_id, 2);
    assert_eq!(snapshot.logic.executions[0].duration_us, 23);
    assert_eq!(
        snapshot.logic.executions[0].status,
        logiksmith_desktop::diagnostics::LogicExecutionStatus::Failed
    );
    assert!(snapshot.logic.executions[0].effects.is_empty());
    assert_eq!(
        snapshot.logic.executions[0]
            .error
            .as_ref()
            .unwrap()
            .category,
        "runtime"
    );
    assert_eq!(
        snapshot.logic.executions[1].status,
        logiksmith_desktop::diagnostics::LogicExecutionStatus::Succeeded
    );
    assert_eq!(snapshot.logic.executions[1].duration_us, 17);
    assert!(snapshot.logic.executions[1].effects.is_empty());
    assert!(snapshot.logic.executions[1].error.is_none());
    assert_eq!(snapshot.logic.executions[1].inputs.len(), 2);
    assert!(snapshot.logic.executions[1].inputs[1].value.is_none());
    assert!(!snapshot.logic.executions[1].inputs[1].valid);
}

#[test]
fn execution_records_use_the_active_document_revision() {
    let mut runtime = make_runtime("function handle(event, input, meta) return nil end");
    runtime.document_revision = 42;
    let mut engine = Engine::new(runtime.engine_config.clone());
    let store = store(&runtime);

    let execution = engine
        .process_input(bool_event("wall_switch", true), MonotonicMs(10))
        .unwrap();
    store.record_execution_at(&execution, MonotonicMs(10), 1, &runtime);

    assert_eq!(store.snapshot().logic.executions[0].logic_revision, 42);
}

#[test]
fn resolves_effect_destinations_and_keeps_snapshots_immutable() {
    let runtime = make_runtime(
        "function handle(event, input, meta) return { outputs = { test_light = true } } end",
    );
    let mut engine = Engine::new(runtime.engine_config.clone());
    let store = store(&runtime);

    let first = engine
        .process_input(bool_event("wall_switch", true), MonotonicMs(10))
        .unwrap();
    store.record_execution_at(&first, MonotonicMs(10), 4, &runtime);
    let later = engine
        .process_input(bool_event("enabled", true), MonotonicMs(20))
        .unwrap();
    store.record_execution_at(&later, MonotonicMs(20), 5, &runtime);

    let executions = store.snapshot().logic.executions;
    assert_eq!(executions[0].trigger.endpoint, "enabled");
    assert_eq!(executions[1].trigger.endpoint, "wall_switch");
    assert_eq!(executions[1].inputs[1].value, None);
    assert_eq!(
        executions[0].inputs[0].value,
        Some(executions[1].trigger.value.clone())
    );
    assert_eq!(executions[1].effects[0].destination, "2/3/52");
}

#[test]
fn retains_newest_fifty_records_in_newest_first_order() {
    let runtime = make_runtime("function handle(event, input, meta) return nil end");
    let mut engine = Engine::new(runtime.engine_config.clone());
    let store = store(&runtime);

    for index in 0..51u64 {
        let execution = engine
            .process_input(
                bool_event("wall_switch", index % 2 == 0),
                MonotonicMs(index + 1),
            )
            .unwrap();
        store.record_execution_at(&execution, MonotonicMs(index + 1), index, &runtime);
    }

    let executions = store.snapshot().logic.executions;
    assert_eq!(executions.len(), 50);
    assert_eq!(executions.first().unwrap().execution_id, 51);
    assert_eq!(executions.last().unwrap().execution_id, 2);
    assert_eq!(executions.first().unwrap().time_ms, 51);
    assert_eq!(executions.last().unwrap().time_ms, 2);
}

#[test]
fn dashboard_json_uses_execution_history_without_legacy_fields() {
    let runtime = make_runtime("function handle(event, input, meta) return nil end");
    let value = serde_json::to_value(store(&runtime).snapshot()).unwrap();
    let logic = value.get("logic").and_then(Value::as_object).unwrap();
    assert!(logic.contains_key("executions"));
    assert!(!logic.contains_key("last_execution"));
    assert!(!logic.contains_key("recent_effects"));
}
