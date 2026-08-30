use logiksmith_core::{
    BlockId, ClockSample, InputEvent, InputUpdate, MonotonicMs, Runtime, TypedValue,
};
use logiksmith_desktop::{
    AutomationBlock, AutomationDocument, AutomationEndpoint, AutomationSignal, BoolValueMessage,
    DptMessage, HttpBinding, HttpPoll, HttpPollValue, KnxBinding, SignalBinding, ValueMessage,
    WebhookBinding, WebhookInput, build_automation,
    diagnostics::{DiagnosticStore, LogicalTriggerRecord},
};
use std::path::PathBuf;

fn make_runtime(source: &str) -> logiksmith_desktop::AutomationRuntime {
    build_automation(AutomationDocument {
        signals: Vec::new(),
        http_polls: Vec::new(),
        webhook_inputs: Vec::new(),
        blocks: vec![AutomationBlock {
            id: "test".to_owned(),
            revision: 1,
            enabled: true,
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
            signal_bindings: Vec::new(),
            http_bindings: Vec::new(),
            webhook_bindings: Vec::new(),
            source: source.to_owned(),
            schedules: Vec::new(),
        }],
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
fn external_source_health_and_execution_origin_are_projected() {
    let runtime = build_automation(AutomationDocument {
        signals: Vec::new(),
        http_polls: vec![HttpPoll {
            name: "forecast".to_owned(),
            url: "https://api.example.test/v1/forecast?token=secret".to_owned(),
            every: "1h".to_owned(),
            timeout: "5s".to_owned(),
            stale_after: "2h".to_owned(),
            headers: Vec::new(),
            values: vec![HttpPollValue {
                name: "today_max".to_owned(),
                dpt: "9.001".to_owned(),
                json_pointer: "/daily/temperature_2m_max/0".to_owned(),
            }],
        }],
        webhook_inputs: vec![WebhookInput {
            name: "override".to_owned(),
            dpt: "1.001".to_owned(),
            json_pointer: "/enabled".to_owned(),
            bearer_token_env: None,
        }],
        blocks: vec![AutomationBlock {
            id: "weather".to_owned(),
            revision: 1,
            enabled: true,
            inputs: vec![
                AutomationEndpoint {
                    name: "today_max".to_owned(),
                    dpt: "9.001".to_owned(),
                },
                AutomationEndpoint {
                    name: "override".to_owned(),
                    dpt: "1.001".to_owned(),
                },
            ],
            outputs: Vec::new(),
            knx_bindings: Vec::new(),
            signal_bindings: Vec::new(),
            http_bindings: vec![HttpBinding {
                endpoint: "today_max".to_owned(),
                source: "today_max".to_owned(),
            }],
            webhook_bindings: vec![WebhookBinding {
                endpoint: "override".to_owned(),
                source: "override".to_owned(),
            }],
            source: "function handle(event, input) return nil end".to_owned(),
            schedules: Vec::new(),
        }],
    })
    .expect("valid external automation");
    let store = store(&runtime);
    let initial = serde_json::to_value(store.snapshot()).unwrap();
    assert_eq!(
        initial["external_inputs"]["http_polls"][0]["url"],
        "https://api.example.test/v1/forecast"
    );
    assert_eq!(
        initial["external_inputs"]["http_polls"][0]["status"],
        "starting"
    );
    assert_eq!(
        initial["external_inputs"]["webhook_inputs"][0]["route"],
        "/api/webhooks/override"
    );

    let sample = ClockSample {
        monotonic_ms: store.now(),
        utc_unix_ms: Some(1_700_000_000_000),
    };
    let site_time = logiksmith_desktop::diagnostics::site_time_snapshot_live(
        &runtime.core_config.site,
        &sample,
    );
    store.set_site_time_sample(sample, site_time);
    store.record_external_poll_attempt("forecast");

    let temperature = TypedValue::temperature(21.75).unwrap();
    store.record_external_poll_success(
        "forecast",
        std::time::Duration::from_secs(7200),
        &[("today_max".to_owned(), temperature)],
    );
    store.record_external_poll_next_attempt("forecast", std::time::Duration::from_secs(3600));
    let healthy = serde_json::to_value(store.snapshot_at(MonotonicMs(5))).unwrap();
    assert_eq!(
        healthy["external_inputs"]["http_polls"][0]["status"],
        "healthy"
    );
    assert_eq!(
        healthy["external_inputs"]["http_polls"][0]["values"][0]["value"]["value"],
        21.75
    );
    let last_attempt = healthy["external_inputs"]["http_polls"][0]["last_attempt_at_ms"]
        .as_u64()
        .unwrap();
    let last_success = healthy["external_inputs"]["http_polls"][0]["last_success_at_ms"]
        .as_u64()
        .unwrap();
    let stale_at = healthy["external_inputs"]["http_polls"][0]["stale_at_ms"]
        .as_u64()
        .unwrap();
    let next_attempt = healthy["external_inputs"]["http_polls"][0]["next_attempt_at_ms"]
        .as_u64()
        .unwrap();
    assert!((1_700_000_000_000..1_700_000_010_000).contains(&last_attempt));
    assert!((1_700_000_000_000..1_700_000_010_000).contains(&last_success));
    assert!((1_700_007_200_000..1_700_007_210_000).contains(&stale_at));
    assert!((1_700_003_600_000..1_700_003_610_000).contains(&next_attempt));
    store.record_external_poll_stale("forecast");
    let stale = serde_json::to_value(store.snapshot()).unwrap();
    assert_eq!(stale["external_inputs"]["http_polls"][0]["status"], "stale");
    assert!(
        !stale["external_inputs"]["http_polls"][0]["values"][0]["valid"]
            .as_bool()
            .unwrap()
    );

    let mut core = Runtime::new(runtime.core_config.clone());
    let execution = core
        .process_input_update_sampled(
            &BlockId::parse("weather").unwrap(),
            "override".parse().unwrap(),
            InputUpdate::Trigger(TypedValue::bool(true)),
            ClockSample {
                monotonic_ms: MonotonicMs(8),
                utc_unix_ms: None,
            },
        )
        .unwrap()
        .pop()
        .unwrap();
    store.record_block_execution_with_origin(
        &execution,
        MonotonicMs(8),
        1,
        &runtime,
        None,
        Some(logiksmith_desktop::diagnostics::ExecutionOrigin::Webhook {
            source: "override".to_owned(),
        }),
    );
    let projected = serde_json::to_value(store.snapshot()).unwrap();
    assert_eq!(
        projected["blocks"][0]["executions"][0]["origin"]["kind"],
        "webhook"
    );
    assert_eq!(
        projected["blocks"][0]["executions"][0]["origin"]["source"],
        "override"
    );
}

#[test]
fn signal_snapshot_and_endpoint_binding_shape_are_serialized() {
    let runtime = build_automation(AutomationDocument {
        signals: vec![AutomationSignal {
            name: "house_occupied".to_owned(),
            dpt: "1.001".to_owned(),
        }],
        http_polls: Vec::new(),
        webhook_inputs: Vec::new(),
        blocks: vec![
            AutomationBlock {
                id: "source".to_owned(),
                revision: 1,
                enabled: true,
                inputs: vec![AutomationEndpoint {
                    name: "switch".to_owned(),
                    dpt: "1.001".to_owned(),
                }],
                outputs: vec![AutomationEndpoint {
                    name: "occupied".to_owned(),
                    dpt: "1.001".to_owned(),
                }],
                knx_bindings: vec![KnxBinding {
                    endpoint: "switch".to_owned(),
                    group_address: "1/1/1".to_owned(),
                }],
                signal_bindings: vec![SignalBinding {
                    endpoint: "occupied".to_owned(),
                    signal: "house_occupied".to_owned(),
                }],
                http_bindings: Vec::new(),
                webhook_bindings: Vec::new(),
                source: "function handle(event, input, meta) return nil end".to_owned(),
                schedules: Vec::new(),
            },
            AutomationBlock {
                id: "consumer".to_owned(),
                revision: 1,
                enabled: true,
                inputs: vec![AutomationEndpoint {
                    name: "occupied".to_owned(),
                    dpt: "1.001".to_owned(),
                }],
                outputs: vec![AutomationEndpoint {
                    name: "light".to_owned(),
                    dpt: "1.001".to_owned(),
                }],
                knx_bindings: vec![KnxBinding {
                    endpoint: "light".to_owned(),
                    group_address: "1/1/2".to_owned(),
                }],
                signal_bindings: vec![SignalBinding {
                    endpoint: "occupied".to_owned(),
                    signal: "house_occupied".to_owned(),
                }],
                http_bindings: Vec::new(),
                webhook_bindings: Vec::new(),
                source: "function handle(event, input, meta) return nil end".to_owned(),
                schedules: Vec::new(),
            },
        ],
    })
    .expect("valid signal automation");
    let json = serde_json::to_value(store(&runtime).snapshot()).unwrap();
    assert_eq!(json["signals"][0]["name"], "house_occupied");
    assert_eq!(json["signals"][0]["status"], "unknown");
    assert!(json["signals"][0]["structuralRevision"].is_string());
    assert_eq!(json["signals"][0]["producer"]["blockId"], "source");
    assert_eq!(json["signals"][0]["consumers"][0]["endpoint"], "occupied");
    assert_eq!(json["blocks"][0]["outputs"][0]["bindingKind"], "signal");
    assert_eq!(json["blocks"][0]["outputs"][0]["signal"], "house_occupied");
    assert_eq!(json["blocks"][1]["inputs"][0]["bindingKind"], "signal");
}

#[test]
fn cascade_execution_diagnostics_keep_signal_provenance() {
    let automation = build_automation(AutomationDocument {
        signals: vec![AutomationSignal {
            name: "occupied".to_owned(),
            dpt: "1.001".to_owned(),
        }],
        http_polls: Vec::new(),
        webhook_inputs: Vec::new(),
        blocks: vec![
            AutomationBlock {
                id: "source".to_owned(),
                revision: 1,
                enabled: true,
                inputs: vec![AutomationEndpoint {
                    name: "trigger".to_owned(),
                    dpt: "1.001".to_owned(),
                }],
                outputs: vec![AutomationEndpoint {
                    name: "out".to_owned(),
                    dpt: "1.001".to_owned(),
                }],
                knx_bindings: vec![KnxBinding {
                    endpoint: "trigger".to_owned(),
                    group_address: "1/2/1".to_owned(),
                }],
                signal_bindings: vec![SignalBinding {
                    endpoint: "out".to_owned(),
                    signal: "occupied".to_owned(),
                }],
                http_bindings: Vec::new(),
                webhook_bindings: Vec::new(),
                source:
                    "function handle(event, input) return { outputs = { out = input.trigger } } end"
                        .to_owned(),
                schedules: Vec::new(),
            },
            AutomationBlock {
                id: "consumer".to_owned(),
                revision: 1,
                enabled: true,
                inputs: vec![AutomationEndpoint {
                    name: "occupied".to_owned(),
                    dpt: "1.001".to_owned(),
                }],
                outputs: Vec::new(),
                knx_bindings: Vec::new(),
                signal_bindings: vec![SignalBinding {
                    endpoint: "occupied".to_owned(),
                    signal: "occupied".to_owned(),
                }],
                http_bindings: Vec::new(),
                webhook_bindings: Vec::new(),
                source: "function handle(event, input) return nil end".to_owned(),
                schedules: Vec::new(),
            },
        ],
    })
    .expect("valid cascade automation");
    let mut engine = Runtime::new(automation.core_config.clone());
    let executions = engine
        .process_input_cascade(
            &BlockId::parse("source").unwrap(),
            bool_event("trigger", true),
            MonotonicMs(10),
        )
        .unwrap();
    assert_eq!(executions.len(), 2);
    let store = store(&automation);
    for execution in &executions {
        store.record_block_execution(execution, MonotonicMs(10), 1, &automation, None);
    }
    store.set_runtime_projection_from_runtime(&engine, MonotonicMs(10));

    let snapshot = store.snapshot_at(MonotonicMs(10));
    assert_eq!(
        snapshot.signals[0].value,
        Some(ValueMessage::Bool(BoolValueMessage {
            kind: "bool".to_owned(),
            value: true,
        }))
    );
    assert_eq!(snapshot.signals[0].recent_changes[0].execution_id, Some(1));
    let consumer = &snapshot.blocks[1].executions[0];
    assert_eq!(consumer.execution_id, 2);
    assert_eq!(consumer.causal_producer_execution_id, Some(1));
    assert_eq!(consumer.causal_signal.as_deref(), Some("occupied"));
    assert_eq!(consumer.causal_links[0].signal.as_deref(), Some("occupied"));
}

#[test]
fn records_zero_effect_and_failure_in_the_owning_block() {
    let runtime = make_runtime("function handle(event, input, meta) return nil end");
    let mut engine = Runtime::new(runtime.core_config.clone());
    let store = store(&runtime);

    let success = engine
        .process_input(
            &BlockId::parse("test").unwrap(),
            bool_event("wall_switch", true),
            MonotonicMs(10),
        )
        .unwrap()
        .unwrap();
    let transition = success.execution.outcome.as_ref().unwrap();
    assert!(transition.outputs.is_empty());
    assert!(transition.state.is_empty());
    assert!(transition.timers.is_empty());
    store.record_block_execution(&success, MonotonicMs(10), 17, &runtime, None);

    let failure_runtime = make_runtime("function handle(event, input, meta) error('boom') end");
    let mut failure_engine = Runtime::new(failure_runtime.core_config.clone());
    let failure = failure_engine
        .process_input(
            &BlockId::parse("test").unwrap(),
            bool_event("wall_switch", true),
            MonotonicMs(10),
        )
        .unwrap()
        .unwrap();
    assert!(failure.execution.outcome.is_err());
    store.record_block_execution(&failure, MonotonicMs(11), 23, &failure_runtime, None);

    let snapshot = store.snapshot_at(MonotonicMs(11));
    assert_eq!(snapshot.blocks.len(), 1);
    assert_eq!(snapshot.blocks[0].executions.len(), 2);
    assert_eq!(snapshot.blocks[0].executions[0].execution_id, 2);
    assert_eq!(snapshot.blocks[0].executions[0].duration_us, 23);
    assert_eq!(
        snapshot.blocks[0].executions[0].status,
        logiksmith_desktop::diagnostics::LogicExecutionStatus::Failed
    );
    assert_eq!(
        snapshot.blocks[0].last_result.as_ref().unwrap().status,
        logiksmith_desktop::diagnostics::LogicExecutionStatus::Failed
    );
}

#[test]
fn retains_newest_fifty_records_per_block() {
    let runtime = make_runtime("function handle(event, input, meta) return nil end");
    let mut engine = Runtime::new(runtime.core_config.clone());
    let store = store(&runtime);
    for index in 0..51u64 {
        let execution = engine
            .process_input(
                &BlockId::parse("test").unwrap(),
                bool_event("wall_switch", index % 2 == 0),
                MonotonicMs(index + 1),
            )
            .unwrap()
            .unwrap();
        store.record_block_execution(&execution, MonotonicMs(index + 1), index, &runtime, None);
    }
    let executions = &store.snapshot().blocks[0].executions;
    assert_eq!(executions.len(), 50);
    assert_eq!(executions.first().unwrap().execution_id, 51);
    assert_eq!(executions.last().unwrap().execution_id, 2);
}

#[test]
fn timer_trigger_logic_revision_is_a_decimal_string() {
    let trigger = LogicalTriggerRecord {
        trigger_type: "timer".to_owned(),
        endpoint: String::new(),
        dpt: DptMessage {
            major: 1,
            subtype: 1,
        },
        value: ValueMessage::Bool(BoolValueMessage {
            kind: "bool".to_owned(),
            value: false,
        }),
        previous: None,
        changed: false,
        rising: false,
        falling: false,
        name: Some("off".to_owned()),
        scheduled_at_ms: Some(1),
        due_at_ms: Some(2),
        fired_at_ms: Some(2),
        late_by_ms: Some(0),
        scheduled_logic_revision: Some(u64::MAX),
        kind: None,
        scheduled_for_utc_ms: None,
        detected_at_utc_ms: None,
        handled_at_utc_ms: None,
        queue_delay_ms: None,
        coalesced_count: None,
        structural_revision: None,
    };

    let value = serde_json::to_value(trigger).unwrap();
    assert_eq!(value["scheduled_logic_revision"], u64::MAX.to_string());
}

#[test]
fn dashboard_json_exposes_ordered_blocks() {
    let runtime = make_runtime("function handle(event, input, meta) return nil end");
    let value = serde_json::to_value(store(&runtime).snapshot()).unwrap();
    let blocks = value
        .get("blocks")
        .and_then(serde_json::Value::as_array)
        .unwrap();
    assert_eq!(blocks[0]["id"], "test");
    assert!(blocks[0].get("state").is_some());
    assert!(blocks[0].get("pending_timers").is_some());
}

#[test]
fn saved_and_active_documents_remain_distinct_until_restart() {
    let runtime = make_runtime("function handle(event, input) return nil end");
    let store = store(&runtime);
    let mut candidate = runtime.document.clone();
    candidate.blocks[0].revision = 2;
    candidate.blocks[0].source = "function handle(event, input) error('new') end".to_owned();
    let structural_revision = runtime.structural_revision;

    store.set_saved_document_state(9, structural_revision, false, &candidate);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.blocks[0].active_revision, 2);
    assert_eq!(snapshot.blocks[0].saved_revision, 2);
    assert_eq!(snapshot.blocks[0].source, candidate.blocks[0].source);
    assert_eq!(store.active_document().blocks[0].revision, 2);

    let mut structural_candidate = candidate.clone();
    structural_candidate.blocks[0].inputs[0].name = "different".to_owned();
    store.set_saved_document_state(
        10,
        structural_revision.wrapping_add(1),
        true,
        &structural_candidate,
    );
    let snapshot = store.snapshot();
    assert_eq!(snapshot.blocks[0].active_revision, 2);
    assert_eq!(snapshot.blocks[0].saved_revision, 2);
    assert_eq!(
        store.active_document().blocks[0].source,
        candidate.blocks[0].source
    );
}
