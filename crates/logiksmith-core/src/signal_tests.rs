fn id(value: &str) -> BlockId {
    value.parse().unwrap()
}

fn endpoint(value: &str) -> EndpointName {
    value.parse().unwrap()
}

fn signal(value: &str) -> SignalName {
    value.parse().unwrap()
}

fn producer(source: &str, enabled: bool) -> BlockConfig {
    let mut block = BlockConfig::new(
        id("producer"),
        enabled,
        vec![
            Endpoint::input(endpoint("trigger"), Dpt::BOOL),
            Endpoint::output(endpoint("out"), Dpt::BOOL),
        ],
        source,
    );
    block
        .signal_bindings
        .push(SignalBinding::new(endpoint("out"), signal("occupied")));
    block
}

fn consumer(
    block_id: &str,
    source: &str,
    enabled: bool,
    input: &str,
    output: Option<(&str, &str)>,
) -> BlockConfig {
    let mut endpoints = vec![Endpoint::input(endpoint(input), Dpt::BOOL)];
    if let Some((name, _)) = output {
        endpoints.push(Endpoint::output(endpoint(name), Dpt::BOOL));
    }
    let mut block = BlockConfig::new(id(block_id), enabled, endpoints, source);
    block
        .signal_bindings
        .push(SignalBinding::new(endpoint(input), signal("occupied")));
    if let Some((name, target)) = output {
        block.signal_bindings.push(SignalBinding::new(
            endpoint(name),
            signal(target),
        ));
    }
    block
}

fn config(blocks: Vec<BlockConfig>, signals: &[(&str, Dpt)]) -> RuntimeConfig {
    RuntimeConfig::with_signals(
        blocks,
        signals
            .iter()
            .map(|(name, dpt)| SignalConfig::new(signal(name), *dpt))
            .collect(),
    )
}

fn input(value: bool) -> InputEvent {
    InputEvent::new(endpoint("trigger"), TypedValue::bool(value))
}

#[test]
fn cascades_chain_in_depth_first_declaration_order() {
    let mut policy = consumer(
        "policy",
        "function handle(event, input) return { outputs = { allowed = input.occupied } } end",
        true,
        "occupied",
        Some(("allowed", "allowed")),
    );
    // Policy's source needs a signal named `occupied`, while its output needs
    // the second global signal.
    let mut hall = BlockConfig::new(
        id("hall"),
        true,
        vec![
            Endpoint::input(endpoint("allowed"), Dpt::BOOL),
            Endpoint::output(endpoint("light"), Dpt::BOOL),
        ],
        "function handle(event, input) return { outputs = { light = input.allowed } } end",
    );
    hall.signal_bindings.push(SignalBinding::new(
        endpoint("allowed"),
        signal("allowed"),
    ));
    let _ = &mut policy;
    let mut runtime = Runtime::new(config(
        vec![
            producer(
                "function handle(event, input) return { outputs = { out = input.trigger } } end",
                true,
            ),
            policy,
            hall,
        ],
        &[("occupied", Dpt::BOOL), ("allowed", Dpt::BOOL)],
    ));

    let executions = runtime
        .process_input_cascade(&id("producer"), input(true), MonotonicMs(10))
        .unwrap();
    assert_eq!(
        executions.iter().map(|item| item.block_id.to_string()).collect::<Vec<_>>(),
        vec!["producer", "policy", "hall"]
    );
    assert_eq!(executions[0].execution.id, Some(1));
    assert_eq!(executions[1].execution.causal_producer, Some(1));
    assert_eq!(executions[2].execution.causal_producer, Some(2));
    assert_eq!(runtime.signal_snapshot(&signal("occupied")).unwrap().value, Some(TypedValue::bool(true)));
    assert_eq!(runtime.signal_snapshot(&signal("allowed")).unwrap().value, Some(TypedValue::bool(true)));
}

#[test]
fn unchanged_signal_refreshes_observation_without_retriggering() {
    let mut runtime = Runtime::new(config(
        vec![producer(
            "function handle(event, input) return { outputs = { out = input.trigger } } end",
            true,
        ), consumer("consumer", "function handle() return nil end", true, "occupied", None)],
        &[("occupied", Dpt::BOOL)],
    ));
    let first = runtime
        .process_input_cascade(&id("producer"), input(true), MonotonicMs(10))
        .unwrap();
    let second = runtime
        .process_input_cascade(&id("producer"), input(true), MonotonicMs(20))
        .unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 1);
    assert!(!second[0].execution.signal_effects[0].changed);
    let snapshot = runtime.signal_snapshot(&signal("occupied")).unwrap();
    assert_eq!(snapshot.observed_at, Some(MonotonicMs(20)));
    assert_eq!(snapshot.changed_at, Some(MonotonicMs(10)));
    assert_eq!(
        runtime
            .block(&id("consumer"))
            .unwrap()
            .snapshot_at(MonotonicMs(20))
            .inputs[0]
            .age_ms,
        Some(0)
    );
}

#[test]
fn disabled_consumer_observes_but_does_not_execute() {
    let mut runtime = Runtime::new(config(
        vec![
            producer(
                "function handle(event, input) return { outputs = { out = input.trigger } } end",
                true,
            ),
            consumer("consumer", "function handle() return nil end", false, "occupied", None),
        ],
        &[("occupied", Dpt::BOOL)],
    ));
    let executions = runtime
        .process_input_cascade(&id("producer"), input(true), MonotonicMs(10))
        .unwrap();
    assert_eq!(executions.len(), 1);
    assert_eq!(
        runtime.block(&id("consumer")).unwrap().snapshot_at(MonotonicMs(10)).known_inputs,
        vec![(endpoint("occupied"), TypedValue::bool(true))]
    );
    assert_eq!(runtime.signal_snapshot(&signal("occupied")).unwrap().status, SignalStatus::Valid);
}

#[test]
fn simulation_proposes_signal_without_mutating_live_state() {
    let runtime = Runtime::new(config(
        vec![producer(
            "function handle(event, input) return { outputs = { out = input.trigger } } end",
            true,
        ), consumer("consumer", "function handle() return nil end", true, "occupied", None)],
        &[("occupied", Dpt::BOOL)],
    ));
    let before = runtime.snapshot();
    let execution = runtime
        .simulate_input(
            &id("producer"),
            SimulationScenario {
                trigger: SimulationTrigger {
                    endpoint: endpoint("trigger"),
                    value: TypedValue::bool(true),
                    previous: None,
                },
                inputs: vec![SimulationInput {
                    endpoint: endpoint("trigger"),
                    value: Some(TypedValue::bool(true)),
                    valid: true,
                    age_ms: Some(0),
                }],
            },
        )
        .unwrap();
    assert_eq!(execution.execution.id, None);
    assert!(execution.execution.signal_effects[0].changed);
    assert_eq!(runtime.snapshot(), before);
}

#[test]
fn fan_out_is_declaration_ordered_and_failure_does_not_stop_siblings() {
    let mut first = consumer(
        "first",
        "function handle(event, input) while true do end end",
        true,
        "occupied",
        Some(("allowed", "first_signal")),
    );
    // Use a separate signal for this branch so its output can be checked for
    // absence after the contained instruction-limit failure.
    let second = consumer(
        "second",
        "function handle(event, input) return { state = { seen = input.occupied } } end",
        true,
        "occupied",
        None,
    );
    first.signal_bindings[1].signal = signal("first_signal");
    let mut runtime = Runtime::new(config(
        vec![
            producer(
                "function handle(event, input) return { outputs = { out = input.trigger } } end",
                true,
            ),
            second,
            first,
        ],
        &[("occupied", Dpt::BOOL), ("first_signal", Dpt::BOOL)],
    ));
    let executions = runtime
        .process_input_cascade(&id("producer"), input(true), MonotonicMs(10))
        .unwrap();
    assert_eq!(
        executions
            .iter()
            .map(|execution| execution.block_id.to_string())
            .collect::<Vec<_>>(),
        vec!["producer", "second", "first"]
    );
    assert!(matches!(
        executions[2].execution.outcome,
        Err(LogicError::InstructionLimit { .. })
    ));
    assert_eq!(runtime.signal_snapshot(&signal("first_signal")).unwrap().value, None);
    assert_eq!(
        runtime.block(&id("second")).unwrap().snapshot_at(MonotonicMs(10)).state["seen"],
        StateValue::Bool(true)
    );
}

#[test]
fn producer_disable_keeps_value_and_marks_status_until_reenabled() {
    let mut runtime = Runtime::new(config(
        vec![producer(
            "function handle(event, input) return { outputs = { out = input.trigger } } end",
            true,
        )],
        &[("occupied", Dpt::BOOL)],
    ));
    runtime
        .process_input_cascade(&id("producer"), input(true), MonotonicMs(10))
        .unwrap();
    runtime
        .activate(RuntimeActivation::single(BlockActivation::enabled(
            id("producer"),
            false,
        )))
        .unwrap();
    let disabled = runtime.signal_snapshot(&signal("occupied")).unwrap();
    assert_eq!(disabled.value, Some(TypedValue::bool(true)));
    assert_eq!(disabled.status, SignalStatus::ProducerDisabled);
    runtime
        .activate(RuntimeActivation::single(BlockActivation::enabled(
            id("producer"),
            true,
        )))
        .unwrap();
    assert_eq!(
        runtime.signal_snapshot(&signal("occupied")).unwrap().status,
        SignalStatus::Valid
    );
}

#[test]
fn signal_dpt_and_binding_references_are_validated() {
    let mut wrong_dpt = producer(
        "function handle(event) return nil end",
        true,
    );
    wrong_dpt.endpoints[1].dpt = Dpt::PERCENT;
    assert!(matches!(
        config(vec![wrong_dpt], &[("occupied", Dpt::BOOL)]).validate(),
        Err(RuntimeConfigError::SignalDptMismatch { .. })
    ));

    let mut unknown = producer(
        "function handle(event) return nil end",
        true,
    );
    unknown.signal_bindings[0].signal = signal("missing");
    assert!(matches!(
        config(vec![unknown], &[("occupied", Dpt::BOOL)]).validate(),
        Err(RuntimeConfigError::UnknownSignal { .. })
    ));

    let mut duplicate = producer(
        "function handle(event) return nil end",
        true,
    );
    duplicate
        .signal_bindings
        .push(SignalBinding::new(endpoint("out"), signal("occupied")));
    assert!(matches!(
        config(vec![duplicate], &[("occupied", Dpt::BOOL)]).validate(),
        Err(RuntimeConfigError::InvalidBlock {
            error: BlockConfigError::DuplicateSignalBinding { .. },
            ..
        })
    ));
}

#[test]
fn duplicate_producers_and_cycles_are_rejected() {
    let mut second = producer(
        "function handle(event) return nil end",
        true,
    );
    second.id = id("second");
    assert!(matches!(
        config(
            vec![producer("function handle(event) return nil end", true), second],
            &[("occupied", Dpt::BOOL)]
        )
        .validate(),
        Err(RuntimeConfigError::DuplicateSignalProducer { .. })
    ));

    let mut left = BlockConfig::new(
        id("left"),
        true,
        vec![
            Endpoint::input(endpoint("in"), Dpt::BOOL),
            Endpoint::output(endpoint("out"), Dpt::BOOL),
        ],
        "function handle() return nil end",
    );
    left.signal_bindings = vec![
        SignalBinding::new(endpoint("in"), signal("right")),
        SignalBinding::new(endpoint("out"), signal("left")),
    ];
    let mut right = left.clone();
    right.id = id("right");
    right.signal_bindings = vec![
        SignalBinding::new(endpoint("in"), signal("left")),
        SignalBinding::new(endpoint("out"), signal("right")),
    ];
    assert!(matches!(
        config(
            vec![left, right],
            &[("left", Dpt::BOOL), ("right", Dpt::BOOL)]
        )
        .validate(),
        Err(RuntimeConfigError::SignalCycle { .. })
    ));
}
