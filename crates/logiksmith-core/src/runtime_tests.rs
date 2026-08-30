
    fn id(value: &str) -> BlockId {
        value.parse().unwrap()
    }

    fn endpoint_name(value: &str) -> EndpointName {
        value.parse().unwrap()
    }

    fn block(id_value: &str, source: &str, enabled: bool) -> BlockConfig {
        BlockConfig::new(
            id(id_value),
            enabled,
            vec![
                Endpoint::input(endpoint_name("input"), Dpt::BOOL),
                Endpoint::output(endpoint_name("light"), Dpt::BOOL),
            ],
            source,
        )
    }

    fn event(value: bool) -> InputEvent {
        InputEvent::new(endpoint_name("input"), TypedValue::bool(value))
    }

    fn schedule_source() -> &'static str {
        "function handle(event) if event.type == 'input' then return { timers = { off = { after = 10 } } } end end"
    }

    #[test]
    fn block_ids_validate_ascii_grammar_and_byte_limit() {
        assert!("a".parse::<BlockId>().is_ok());
        assert!("a_2".parse::<BlockId>().is_ok());
        assert!("a".repeat(MAX_BLOCK_ID_BYTES).parse::<BlockId>().is_ok());
        assert!(
            "a".repeat(MAX_BLOCK_ID_BYTES + 1)
                .parse::<BlockId>()
                .is_err()
        );
        for invalid in ["", "A", "1abc", "a-b", "a.b", "a B", "ą"] {
            assert!(invalid.parse::<BlockId>().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn runtime_requires_unique_nonempty_and_at_most_64_blocks() {
        assert!(matches!(
            RuntimeConfig::new(Vec::new()).validate(),
            Err(RuntimeConfigError::Empty)
        ));
        assert!(matches!(
            RuntimeConfig::new(vec![
                block("same", "function handle() end", true),
                block("same", "function handle() end", true)
            ])
            .validate(),
            Err(RuntimeConfigError::DuplicateId(_))
        ));
        let blocks = (0..MAX_BLOCKS)
            .map(|index| block(&format!("b{index}"), "function handle() end", true))
            .collect();
        assert!(RuntimeConfig::new(blocks).validate().is_ok());
        let blocks = (0..=MAX_BLOCKS)
            .map(|index| block(&format!("b{index}"), "function handle() end", true))
            .collect();
        assert!(matches!(
            RuntimeConfig::new(blocks).validate(),
            Err(RuntimeConfigError::TooMany {
                actual: 65,
                maximum: 64
            })
        ));
    }

    #[test]
    fn local_endpoint_names_and_timer_names_are_isolated() {
        let mut runtime = Runtime::new(RuntimeConfig::new(vec![
            block("alpha", schedule_source(), true),
            block("beta", schedule_source(), true),
        ]));
        runtime
            .process_input(&id("alpha"), event(true), MonotonicMs(1))
            .unwrap();
        runtime
            .process_input(&id("beta"), event(true), MonotonicMs(1))
            .unwrap();
        assert_eq!(
            runtime
                .block(&id("alpha"))
                .unwrap()
                .snapshot_at(MonotonicMs(1))
                .pending_timers
                .len(),
            1
        );
        assert_eq!(
            runtime
                .block(&id("beta"))
                .unwrap()
                .snapshot_at(MonotonicMs(1))
                .pending_timers
                .len(),
            1
        );
        assert_eq!(runtime.next_timer_deadline(), Some(MonotonicMs(11)));
    }

    #[test]
    fn equal_deadline_timers_are_ordered_by_block_id_then_timer_name() {
        let source = "function handle(event) if event.type == 'input' then return { timers = { z = { after = 10 }, a = { after = 10 } } } end end";
        let mut runtime = Runtime::new(RuntimeConfig::new(vec![
            block("zeta", source, true),
            block("alpha", source, true),
        ]));
        runtime
            .process_input(&id("zeta"), event(true), MonotonicMs(1))
            .unwrap();
        runtime
            .process_input(&id("alpha"), event(true), MonotonicMs(1))
            .unwrap();

        // alpha wins over zeta despite being declared second; within alpha,
        // timer `a` wins over `z`.
        let first = runtime
            .process_next_due_timer(MonotonicMs(11))
            .unwrap()
            .unwrap();
        assert_eq!(first.block_id, id("alpha"));
        assert!(
            matches!(first.execution.trigger, Trigger::Timer(TimerTrigger { name, .. }) if name == TimerName::new("a").unwrap())
        );
        let second = runtime
            .process_next_due_timer(MonotonicMs(11))
            .unwrap()
            .unwrap();
        assert_eq!(second.block_id, id("alpha"));
        assert!(
            matches!(second.execution.trigger, Trigger::Timer(TimerTrigger { name, .. }) if name == TimerName::new("z").unwrap())
        );
        let third = runtime
            .process_next_due_timer(MonotonicMs(11))
            .unwrap()
            .unwrap();
        assert_eq!(third.block_id, id("zeta"));
    }

    #[test]
    fn failed_block_execution_is_contained_and_next_block_succeeds() {
        let mut runtime = Runtime::new(RuntimeConfig::new(vec![
            block("broken", "function handle() while true do end end", true),
            block(
                "healthy",
                "function handle() return { outputs = { light = true } } end",
                true,
            ),
        ]));
        let failed = runtime
            .process_input(&id("broken"), event(true), MonotonicMs(1))
            .unwrap()
            .unwrap();
        assert!(matches!(
            failed.execution.outcome,
            Err(LogicError::InstructionLimit { .. })
        ));
        let healthy = runtime
            .process_input(&id("healthy"), event(true), MonotonicMs(1))
            .unwrap()
            .unwrap();
        assert_eq!(healthy.block_id, id("healthy"));
        assert!(healthy.execution.outcome.is_ok());
    }

    #[test]
    fn disabled_blocks_observe_without_execution_and_reenable_quietly() {
        let source = "function handle(event, input) return { state = { seen = input.input }, timers = { off = { after = 10 } } } end";
        let mut runtime = Runtime::new(RuntimeConfig::new(vec![block("quiet", source, true)]));
        runtime
            .process_input(&id("quiet"), event(true), MonotonicMs(1))
            .unwrap();
        let disabled = runtime
            .activate(RuntimeActivation::single(BlockActivation::enabled(
                id("quiet"),
                false,
            )))
            .unwrap();
        assert_eq!(
            disabled.blocks[0].cancelled_timers,
            vec![TimerName::new("off").unwrap()]
        );
        let snapshot = runtime
            .block(&id("quiet"))
            .unwrap()
            .snapshot_at(MonotonicMs(2));
        assert!(!snapshot.enabled);
        assert_eq!(snapshot.state["seen"], StateValue::Bool(true));
        assert!(snapshot.pending_timers.is_empty());
        assert_eq!(
            runtime
                .process_input(&id("quiet"), event(false), MonotonicMs(2))
                .unwrap(),
            None
        );
        assert_eq!(
            runtime
                .block(&id("quiet"))
                .unwrap()
                .snapshot_at(MonotonicMs(2))
                .known_inputs,
            vec![(endpoint_name("input"), TypedValue::bool(false))]
        );
        runtime
            .activate(RuntimeActivation::single(BlockActivation::enabled(
                id("quiet"),
                true,
            )))
            .unwrap();
        // Enabling itself does not invoke Lua or restore the cancelled timer.
        assert!(runtime.next_timer_deadline().is_none());
    }

    #[test]
    fn source_activation_is_atomic_and_cancels_only_own_timers() {
        let mut runtime = Runtime::new(RuntimeConfig::new(vec![
            block("alpha", schedule_source(), true),
            block("beta", schedule_source(), true),
        ]));
        runtime
            .process_input(&id("alpha"), event(true), MonotonicMs(1))
            .unwrap();
        runtime
            .process_input(&id("beta"), event(true), MonotonicMs(1))
            .unwrap();
        let before_alpha = runtime.block(&id("alpha")).unwrap().active_logic_revision();
        let before_beta = runtime.block(&id("beta")).unwrap().active_logic_revision();
        let error = runtime.activate(RuntimeActivation::new(vec![
            BlockActivation::source(id("alpha"), "function handle() return nil end"),
            BlockActivation::source(id("beta"), "function handle( "),
        ]));
        assert!(
            matches!(error, Err(ActivationError::InvalidSource { block_id, .. }) if block_id == id("beta"))
        );
        assert_eq!(
            runtime.block(&id("alpha")).unwrap().active_logic_revision(),
            before_alpha
        );
        assert_eq!(
            runtime.block(&id("beta")).unwrap().active_logic_revision(),
            before_beta
        );
        assert_eq!(
            runtime
                .block(&id("alpha"))
                .unwrap()
                .snapshot_at(MonotonicMs(1))
                .pending_timers
                .len(),
            1
        );
        assert_eq!(
            runtime
                .block(&id("beta"))
                .unwrap()
                .snapshot_at(MonotonicMs(1))
                .pending_timers
                .len(),
            1
        );

        let result = runtime
            .activate(RuntimeActivation::single(BlockActivation::source(
                id("alpha"),
                "function handle() return nil end",
            )))
            .unwrap();
        assert_eq!(result.blocks[0].cancelled_timers.len(), 1);
        assert!(
            runtime
                .block(&id("alpha"))
                .unwrap()
                .snapshot_at(MonotonicMs(1))
                .pending_timers
                .is_empty()
        );
        assert_eq!(
            runtime
                .block(&id("beta"))
                .unwrap()
                .snapshot_at(MonotonicMs(1))
                .pending_timers
                .len(),
            1
        );
    }

    #[test]
    fn simulation_is_immutable_and_allowed_for_disabled_blocks() {
        let runtime = Runtime::new(RuntimeConfig::new(vec![block(
            "sim",
            "function handle(event) return { state = { value = true }, outputs = { light = true } } end",
            false,
        )]));
        let before = runtime.snapshot();
        let simulation = runtime
            .simulate_input(
                &id("sim"),
                SimulationScenario {
                    trigger: SimulationTrigger {
                        endpoint: endpoint_name("input"),
                        value: TypedValue::bool(true),
                        previous: None,
                    },
                    inputs: vec![SimulationInput {
                        endpoint: endpoint_name("input"),
                        value: Some(TypedValue::bool(true)),
                        valid: true,
                        age_ms: Some(0),
                    }],
                },
            )
            .unwrap();
        assert_eq!(simulation.block_id, id("sim"));
        assert!(simulation.execution.outcome.is_ok());
        assert_eq!(runtime.snapshot(), before);
    }

    #[test]
    fn runtime_supplied_source_simulation_does_not_change_live_block() {
        let runtime = Runtime::new(RuntimeConfig::new(vec![block(
            "draft",
            "function handle(event) return { outputs = { light = false } } end",
            true,
        )]));
        let before = runtime.snapshot();
        let execution = runtime
            .simulate_input_with_source(
                &id("draft"),
                "function handle(event) return { outputs = { light = true } } end",
                SimulationScenario {
                    trigger: SimulationTrigger {
                        endpoint: endpoint_name("input"),
                        value: TypedValue::bool(true),
                        previous: None,
                    },
                    inputs: vec![SimulationInput {
                        endpoint: endpoint_name("input"),
                        value: Some(TypedValue::bool(true)),
                        valid: true,
                        age_ms: Some(0),
                    }],
                },
                TransientState::new(),
                Vec::new(),
                MonotonicMs(4),
            )
            .unwrap();
        assert!(execution.execution.outcome.is_ok());
        assert_eq!(runtime.snapshot(), before);
    }

    #[test]
    fn equal_runtime_timestamps_are_valid_but_lower_timestamps_are_rejected() {
        let mut runtime = Runtime::new(RuntimeConfig::new(vec![
            block("alpha", "function handle() end", true),
            block("beta", "function handle() end", true),
        ]));
        runtime
            .observe_input(
                &id("alpha"),
                InputObservation::new(endpoint_name("input"), TypedValue::bool(true)),
                MonotonicMs(10),
            )
            .unwrap();
        runtime
            .observe_input(
                &id("beta"),
                InputObservation::new(endpoint_name("input"), TypedValue::bool(true)),
                MonotonicMs(10),
            )
            .unwrap();
        assert!(matches!(
            runtime.observe_input(
                &id("alpha"),
                InputObservation::new(endpoint_name("input"), TypedValue::bool(false)),
                MonotonicMs(9),
            ),
            Err(RuntimeEventError::TimeWentBackwards { .. })
        ));
        assert_eq!(
            runtime
                .block(&id("alpha"))
                .unwrap()
                .snapshot_at(MonotonicMs(10))
                .known_inputs[0]
                .1,
            TypedValue::bool(true)
        );
    }
