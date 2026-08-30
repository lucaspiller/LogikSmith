#[test]
fn local_config_example_contains_the_required_bridge_section() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/local.toml.example");
    let source = fs::read_to_string(path).expect("local config example should be present");
    let config: RawConfig = toml::from_str(&source).expect("local config example should parse");
    assert_eq!(config.bridge.python, ".venv/bin/python");
}

fn block(id: &str, input_address: &str, output_address: &str) -> AutomationBlock {
    AutomationBlock {
        id: id.to_owned(),
        revision: 1,
        enabled: true,
        inputs: vec![AutomationEndpoint {
            name: "input".to_owned(),
            dpt: "1.001".to_owned(),
        }],
        outputs: vec![AutomationEndpoint {
            name: "light".to_owned(),
            dpt: "1.001".to_owned(),
        }],
        knx_bindings: vec![
            KnxBinding {
                endpoint: "input".to_owned(),
                group_address: input_address.to_owned(),
            },
            KnxBinding {
                endpoint: "light".to_owned(),
                group_address: output_address.to_owned(),
            },
        ],
        signal_bindings: Vec::new(),
        http_bindings: Vec::new(),
        webhook_bindings: Vec::new(),
        source: "function handle(event, input) return nil end".to_owned(),
        schedules: Vec::new(),
    }
}

#[cfg(not(feature = "http-inputs"))]
#[test]
fn stripped_build_rejects_http_poll_configuration() {
    let errors = build_automation(AutomationDocument {
        signals: Vec::new(),
        http_polls: vec![HttpPoll {
            name: "weather".to_owned(),
            url: "https://example.invalid/weather".to_owned(),
            every: "60s".to_owned(),
            timeout: "1s".to_owned(),
            stale_after: "5m".to_owned(),
            headers: Vec::new(),
            values: vec![HttpPollValue {
                name: "outside".to_owned(),
                dpt: "9.001".to_owned(),
                json_pointer: "/outside".to_owned(),
            }],
        }],
        webhook_inputs: Vec::new(),
        blocks: vec![block("weather_block", "1/1/1", "1/2/1")],
    })
    .expect_err("HTTP input must be unavailable in a stripped build");
    assert!(
        errors.iter().any(|error| {
            error.path == "http_polls" && error.message.contains("feature_disabled")
        })
    );
}

#[cfg(not(feature = "webhook-inputs"))]
#[test]
fn stripped_build_rejects_webhook_configuration() {
    let errors = build_automation(AutomationDocument {
        signals: Vec::new(),
        http_polls: Vec::new(),
        webhook_inputs: vec![WebhookInput {
            name: "doorbell".to_owned(),
            dpt: "1.001".to_owned(),
            json_pointer: "/pressed".to_owned(),
            bearer_token_env: None,
        }],
        blocks: vec![block("doorbell_block", "1/1/2", "1/2/2")],
    })
    .expect_err("webhook input must be unavailable in a stripped build");
    assert!(errors.iter().any(|error| {
        error.path == "webhook_inputs" && error.message.contains("feature_disabled")
    }));
}

#[test]
fn nested_document_supports_sixty_four_blocks_and_rejects_sixty_five() {
    let document = AutomationDocument {
        signals: Vec::new(),
        http_polls: Vec::new(),
        webhook_inputs: Vec::new(),
        blocks: (0..64)
            .map(|index| {
                block(
                    &format!("block_{index}"),
                    &format!("1/1/{}", index + 1),
                    &format!("1/2/{}", index + 1),
                )
            })
            .collect(),
    };
    let runtime = build_automation(document.clone()).unwrap();
    assert_eq!(runtime.blocks.len(), 64);
    let too_many = AutomationDocument {
        signals: Vec::new(),
        http_polls: Vec::new(),
        webhook_inputs: Vec::new(),
        blocks: (0..65)
            .map(|index| {
                block(
                    &format!("block_{index}"),
                    &format!("2/1/{}", index + 1),
                    &format!("2/2/{}", index + 1),
                )
            })
            .collect(),
    };
    assert!(build_automation(too_many).is_err());
}

#[test]
fn structural_revision_ignores_source_enabled_and_persisted_block_revision() {
    let mut first = AutomationDocument {
        signals: Vec::new(),
        http_polls: Vec::new(),
        webhook_inputs: Vec::new(),
        blocks: vec![block("one", "6/1/1", "6/2/1")],
    };
    let mut second = first.clone();
    second.blocks[0].source = "function handle(event) return nil end".to_owned();
    second.blocks[0].enabled = false;
    second.blocks[0].revision = 91;
    assert_eq!(structural_revision(&first), structural_revision(&second));

    first.blocks[0].inputs[0].dpt = "5.001".to_owned();
    assert_ne!(structural_revision(&first), structural_revision(&second));
}

#[test]
fn schedule_duration_order_and_interval_weekdays_are_rejected() {
    assert_eq!(parse_duration_seconds("1h30m", false), Ok(5_400));
    assert_eq!(parse_duration_seconds("30m1h", false), Err(()));

    let schedule = AutomationSchedule {
        name: "heartbeat".to_owned(),
        enabled: true,
        kind: "interval".to_owned(),
        at: None,
        every: Some("60s".to_owned()),
        offset: None,
        anchor: None,
        weekdays: Some(vec!["mon".to_owned()]),
        extra: Default::default(),
    };
    let mut errors = Vec::new();
    let _ = schedule_rule(&schedule, "blocks[0].schedules[0]", &mut errors);
    assert!(errors.iter().any(|error| error.path.ends_with(".weekdays")));
}

#[test]
fn schedule_only_block_does_not_need_an_input() {
    let mut schedule_only = block("schedule_only", "2/1/1", "2/2/1");
    schedule_only.inputs.clear();
    schedule_only
        .knx_bindings
        .retain(|binding| binding.endpoint != "input");
    schedule_only.schedules = vec![AutomationSchedule {
        name: "turn_on".to_owned(),
        enabled: true,
        kind: "fixed".to_owned(),
        at: Some("18:00".to_owned()),
        every: None,
        offset: None,
        anchor: None,
        weekdays: None,
        extra: Default::default(),
    }];

    assert!(
        build_automation(AutomationDocument {
            signals: Vec::new(),
            http_polls: Vec::new(),
            webhook_inputs: Vec::new(),
            blocks: vec![schedule_only],
        })
        .is_ok()
    );
}

#[test]
fn schedule_only_toml_may_omit_inputs() {
    let document: AutomationDocument = toml::from_str(
        r#"
[[blocks]]
id = "schedule_only"
enabled = true
source = "function handle() return nil end"

[[blocks.schedules]]
name = "turn_on"
enabled = true
kind = "fixed"
at = "18:00"

[[blocks.outputs]]
name = "light"
dpt = "1.001"

[[blocks.knx_bindings]]
endpoint = "light"
group_address = "2/2/1"
"#,
    )
    .expect("schedule-only TOML should load without an inputs field");

    assert!(document.blocks[0].inputs.is_empty());
    assert!(build_automation(document).is_ok());
}

#[test]
fn shared_same_dpt_address_fans_out_in_declaration_order() {
    let runtime = build_automation(AutomationDocument {
        signals: Vec::new(),
        http_polls: Vec::new(),
        webhook_inputs: Vec::new(),
        blocks: vec![
            block("first", "3/1/1", "3/2/1"),
            block("second", "3/1/1", "3/2/2"),
        ],
    })
    .unwrap();
    let bindings = runtime
        .address_to_inputs
        .get(&GroupAddress::parse("3/1/1").unwrap())
        .unwrap();
    assert_eq!(
        bindings
            .iter()
            .map(|binding| binding.block_id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(runtime.address_dpts.len(), 3);
}

#[test]
fn cross_block_dpt_conflict_and_local_duplicate_address_are_rejected() {
    let mut conflicting = block("second", "4/1/1", "4/2/1");
    conflicting.inputs[0].dpt = "5.001".to_owned();
    let errors = build_automation(AutomationDocument {
        signals: Vec::new(),
        http_polls: Vec::new(),
        webhook_inputs: Vec::new(),
        blocks: vec![block("first", "4/1/1", "4/2/2"), conflicting],
    })
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.path == "blocks[1].knx_bindings[0].group_address")
    );

    let mut duplicate = block("first", "5/1/1", "5/2/1");
    duplicate.knx_bindings[1].group_address = "5/1/1".to_owned();
    let errors = build_automation(AutomationDocument {
        signals: Vec::new(),
        http_polls: Vec::new(),
        webhook_inputs: Vec::new(),
        blocks: vec![duplicate],
    })
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.path == "blocks[0].knx_bindings[1].group_address")
    );
}

#[test]
fn signals_round_trip_and_share_endpoint_binding_validation() {
    let mut producer = block("producer", "7/1/1", "7/2/1");
    producer
        .knx_bindings
        .retain(|binding| binding.endpoint != "light");
    producer.signal_bindings = vec![SignalBinding {
        endpoint: "light".to_owned(),
        signal: "house_occupied".to_owned(),
    }];
    let document = AutomationDocument {
        signals: vec![AutomationSignal {
            name: "house_occupied".to_owned(),
            dpt: "1.001".to_owned(),
        }],
        http_polls: Vec::new(),
        webhook_inputs: Vec::new(),
        blocks: vec![producer],
    };
    let runtime = build_automation(document.clone()).expect("valid signal binding");
    assert_eq!(runtime.signals[0].name.as_str(), "house_occupied");
    assert_eq!(runtime.output_to_signal.len(), 1);
    assert_eq!(runtime.signal_to_inputs.len(), 0);

    let path = std::env::temp_dir().join(format!(
        "logiksmith-signals-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, serialize_automation(&document, 0).unwrap()).unwrap();
    assert_eq!(load_automation(&path).unwrap().0, document);
    let _ = fs::remove_file(path);
}

#[test]
fn signals_reject_unknown_or_conflicting_bindings_and_duplicate_producers() {
    let mut first = block("first", "8/1/1", "8/2/1");
    first
        .knx_bindings
        .retain(|binding| binding.endpoint != "light");
    first.signal_bindings = vec![SignalBinding {
        endpoint: "light".to_owned(),
        signal: "allowed".to_owned(),
    }];
    let mut second = block("second", "8/1/2", "8/2/2");
    second
        .knx_bindings
        .retain(|binding| binding.endpoint != "light");
    second.signal_bindings = vec![SignalBinding {
        endpoint: "light".to_owned(),
        signal: "allowed".to_owned(),
    }];
    let errors = build_automation(AutomationDocument {
        signals: vec![AutomationSignal {
            name: "allowed".to_owned(),
            dpt: "1.001".to_owned(),
        }],
        http_polls: Vec::new(),
        webhook_inputs: Vec::new(),
        blocks: vec![first, second],
    })
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.path == "blocks[1].signal_bindings[0].signal")
    );

    let mut mismatched = block("mismatched", "9/1/1", "9/2/1");
    mismatched
        .knx_bindings
        .retain(|binding| binding.endpoint != "light");
    mismatched.signal_bindings = vec![SignalBinding {
        endpoint: "light".to_owned(),
        signal: "unknown".to_owned(),
    }];
    let errors = build_automation(AutomationDocument {
        signals: vec![AutomationSignal {
            name: "allowed".to_owned(),
            dpt: "1.001".to_owned(),
        }],
        http_polls: Vec::new(),
        webhook_inputs: Vec::new(),
        blocks: vec![mismatched],
    })
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.path == "blocks[0].signal_bindings[0].signal")
    );
}

#[test]
fn legacy_top_level_shape_reports_migration_fields() {
    let path = std::env::temp_dir().join(format!("logiksmith-legacy-{}.toml", std::process::id()));
    fs::write(&path, "[logic]\nsource = \"function handle() end\"\n").unwrap();
    let error = load_automation(&path).unwrap_err();
    assert!(
        matches!(error, AutomationFileError::Invalid(errors) if errors.iter().any(|error| error.path == "logic"))
    );
    let _ = fs::remove_file(path);
}
