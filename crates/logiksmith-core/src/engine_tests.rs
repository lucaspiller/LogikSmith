use std::collections::BTreeMap;

    fn name(value: &str) -> EndpointName {
        value.parse().unwrap()
    }

    fn endpoint(value: &str, direction: EndpointDirection, dpt: Dpt) -> Endpoint {
        Endpoint::new(name(value), direction, dpt)
    }

    fn source() -> &'static str {
        "function handle(event, input)\n  if event.input == 'wall_switch' and event.value == true then\n    return { outputs = { test_light = true, dimmer_output = input.dimmer_level or 0 } }\n  end\nend"
    }

    fn config() -> EngineConfig {
        EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("dimmer_level", EndpointDirection::Input, Dpt::PERCENT),
                endpoint("unused_input", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
                endpoint("dimmer_output", EndpointDirection::Output, Dpt::PERCENT),
            ],
            source(),
        )
    }

    fn trigger(value: bool) -> InputEvent {
        InputEvent::new(name("wall_switch"), TypedValue::bool(value))
    }

    fn simulation_input(
        endpoint: &str,
        value: Option<TypedValue>,
        valid: bool,
        age_ms: Option<u64>,
    ) -> SimulationInput {
        SimulationInput {
            endpoint: name(endpoint),
            value,
            valid,
            age_ms,
        }
    }

    fn simulation_scenario(
        value: bool,
        previous: Option<bool>,
        inputs: Vec<SimulationInput>,
    ) -> SimulationScenario {
        SimulationScenario {
            trigger: SimulationTrigger {
                endpoint: name("wall_switch"),
                value: TypedValue::bool(value),
                previous: previous.map(TypedValue::bool),
            },
            inputs,
        }
    }

    fn at(now: u64) -> MonotonicMs {
        MonotonicMs(now)
    }

    fn run(engine: &mut Engine, value: bool, now: u64) -> Execution {
        engine.process_input(trigger(value), at(now)).unwrap()
    }

    fn effects(execution: &Execution) -> &Vec<Effect> {
        &execution.outcome.as_ref().unwrap().outputs
    }

    #[test]
    fn endpoint_names_and_values_are_typed() {
        assert!("wall_switch".parse::<EndpointName>().is_ok());
        assert!("Wall_switch".parse::<EndpointName>().is_err());
        assert_eq!(Dpt::BOOL.to_string(), "1.001");
        assert_eq!(Dpt::PERCENT.to_string(), "5.001");
        assert!(TypedValue::new(Dpt::BOOL, Value::Percent(42)).is_err());
        assert!(TypedValue::new(Dpt::PERCENT, Value::Percent(101)).is_err());
    }

    #[test]
    fn valid_source_loads_and_required_handler_is_checked() {
        let engine = Engine::try_new(config()).unwrap();
        assert_eq!(
            engine.active_logic_revision(),
            LogicProgram::new(source()).revision
        );
        assert!(matches!(
            Engine::validate_source("function nope() end"),
            Err(LogicError::Load { .. })
        ));
    }

    #[test]
    fn syntax_and_empty_or_oversized_sources_are_rejected() {
        assert!(matches!(
            Engine::validate_source("function handle( "),
            Err(LogicError::Syntax { line: Some(_), .. })
        ));
        assert!(matches!(
            Engine::validate_source("   "),
            Err(LogicError::EmptySource)
        ));
        let oversized = "x".repeat(MAX_LOGIC_SOURCE_BYTES + 1);
        assert!(matches!(
            Engine::validate_source(&oversized),
            Err(LogicError::SourceTooLarge { .. })
        ));
    }

    #[test]
    fn observations_update_snapshot_without_execution() {
        let mut engine = Engine::new(config());
        engine
            .observe_input(
                InputObservation::new(name("dimmer_level"), TypedValue::percent(42).unwrap()),
                MonotonicMs(10),
            )
            .unwrap();
        assert_eq!(
            engine.known_input_values(),
            vec![(name("dimmer_level"), TypedValue::percent(42).unwrap())]
        );
    }

    #[test]
    fn triggering_value_is_in_snapshot_and_outputs_are_declaration_ordered() {
        let mut engine = Engine::new(config());
        engine
            .observe_input(
                InputObservation::new(name("dimmer_level"), TypedValue::percent(42).unwrap()),
                MonotonicMs(10),
            )
            .unwrap();
        let execution = run(&mut engine, true, 20);
        assert_eq!(effects(&execution).len(), 2);
        assert_eq!(
            effects(&execution).as_slice(),
            vec![
                OutputEffect {
                    endpoint: name("test_light"),
                    value: TypedValue::bool(true),
                },
                OutputEffect {
                    endpoint: name("dimmer_output"),
                    value: TypedValue::percent(42).unwrap(),
                },
            ]
        );
        assert!(
            engine
                .known_input_values()
                .contains(&(name("wall_switch"), TypedValue::bool(true)))
        );
        assert_eq!(execution.inputs[0].age_ms, Some(0));
        assert_eq!(execution.inputs[1].age_ms, Some(10));
    }

    #[test]
    fn valid_simulation_uses_complete_ordered_snapshot_and_does_not_mutate_engine() {
        let engine = Engine::new(config());
        let before = engine.snapshot();
        let scenario = simulation_scenario(
            true,
            Some(false),
            vec![
                simulation_input("unused_input", None, false, None),
                simulation_input(
                    "dimmer_level",
                    Some(TypedValue::percent(42).unwrap()),
                    true,
                    Some(25),
                ),
                simulation_input("wall_switch", Some(TypedValue::bool(true)), true, Some(0)),
            ],
        );
        let first = engine.simulate_input(scenario.clone()).unwrap();
        let repeated = engine.simulate_input(scenario).unwrap();

        assert_eq!(first, repeated);
        assert_eq!(engine.snapshot(), before);
        assert_eq!(
            first
                .inputs
                .iter()
                .map(|input| input.endpoint.as_str())
                .collect::<Vec<_>>(),
            vec!["wall_switch", "dimmer_level", "unused_input"]
        );
        assert_eq!(first.inputs[0].age_ms, Some(0));
        assert_eq!(first.inputs[1].age_ms, Some(25));
        assert_eq!(first.inputs[2].value, None);
        assert_eq!(effects(&first).len(), 2);
    }

    #[test]
    fn simulation_derives_boolean_edges_from_optional_previous_value() {
        let engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("enabled", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) return nil end",
        ));
        let inputs = |value| {
            vec![
                simulation_input("wall_switch", Some(TypedValue::bool(value)), true, Some(0)),
                simulation_input("enabled", None, false, None),
            ]
        };

        let unknown = engine
            .simulate_input(simulation_scenario(false, None, inputs(false)))
            .unwrap();
        assert!(!unknown.trigger.changed);
        assert!(!unknown.trigger.rising);
        assert!(!unknown.trigger.falling);

        let rising = engine
            .simulate_input(simulation_scenario(true, Some(false), inputs(true)))
            .unwrap();
        assert!(rising.trigger.changed);
        assert!(rising.trigger.rising);
        assert!(!rising.trigger.falling);

        let falling = engine
            .simulate_input(simulation_scenario(false, Some(true), inputs(false)))
            .unwrap();
        assert!(falling.trigger.changed);
        assert!(!falling.trigger.rising);
        assert!(falling.trigger.falling);

        let repeated = engine
            .simulate_input(simulation_scenario(true, Some(true), inputs(true)))
            .unwrap();
        assert!(!repeated.trigger.changed);
        assert!(!repeated.trigger.rising);
        assert!(!repeated.trigger.falling);
    }

    #[test]
    fn simulation_preserves_percentage_values_and_output_order() {
        let engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("level", EndpointDirection::Input, Dpt::PERCENT),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
                endpoint("dimmer_output", EndpointDirection::Output, Dpt::PERCENT),
            ],
            "function handle(event, input)\n  if event.rising then return { outputs = { dimmer_output = input.level, test_light = true } } end\nend",
        ));
        let execution = engine
            .simulate_input(simulation_scenario(
                true,
                Some(false),
                vec![
                    simulation_input(
                        "level",
                        Some(TypedValue::percent(73).unwrap()),
                        true,
                        Some(8),
                    ),
                    simulation_input("wall_switch", Some(TypedValue::bool(true)), true, Some(0)),
                ],
            ))
            .unwrap();
        assert_eq!(
            effects(&execution),
            &vec![
                OutputEffect {
                    endpoint: name("test_light"),
                    value: TypedValue::bool(true),
                },
                OutputEffect {
                    endpoint: name("dimmer_output"),
                    value: TypedValue::percent(73).unwrap(),
                },
            ]
        );
    }

    #[test]
    fn simulation_rejects_incomplete_duplicate_unknown_and_malformed_inputs() {
        let engine = Engine::new(config());
        let valid_wall =
            || simulation_input("wall_switch", Some(TypedValue::bool(true)), true, Some(0));
        let valid_dimmer = || {
            simulation_input(
                "dimmer_level",
                Some(TypedValue::percent(50).unwrap()),
                true,
                Some(1),
            )
        };
        let invalid_unused = || simulation_input("unused_input", None, false, None);

        let unknown = simulation_scenario(
            true,
            None,
            vec![
                valid_wall(),
                valid_dimmer(),
                invalid_unused(),
                simulation_input("unknown", None, false, None),
            ],
        );
        assert!(matches!(
            engine.simulate_input(unknown),
            Err(SimulationError::UnknownEndpoint(endpoint)) if endpoint == name("unknown")
        ));

        let duplicate = simulation_scenario(
            true,
            None,
            vec![valid_wall(), valid_wall(), valid_dimmer(), invalid_unused()],
        );
        assert!(matches!(
            engine.simulate_input(duplicate),
            Err(SimulationError::DuplicateInput(endpoint)) if endpoint == name("wall_switch")
        ));

        let missing = simulation_scenario(true, None, vec![valid_wall(), valid_dimmer()]);
        assert!(matches!(
            engine.simulate_input(missing),
            Err(SimulationError::MissingInput(endpoint)) if endpoint == name("unused_input")
        ));

        let missing_value = simulation_scenario(
            true,
            None,
            vec![
                simulation_input("wall_switch", None, true, Some(0)),
                valid_dimmer(),
                invalid_unused(),
            ],
        );
        assert!(matches!(
            engine.simulate_input(missing_value),
            Err(SimulationError::MissingValue(endpoint)) if endpoint == name("wall_switch")
        ));

        let unexpected_value = simulation_scenario(
            true,
            None,
            vec![
                valid_wall(),
                valid_dimmer(),
                simulation_input("unused_input", Some(TypedValue::bool(false)), false, None),
            ],
        );
        assert!(matches!(
            engine.simulate_input(unexpected_value),
            Err(SimulationError::UnexpectedValue(endpoint)) if endpoint == name("unused_input")
        ));

        let missing_age = simulation_scenario(
            true,
            None,
            vec![
                simulation_input("wall_switch", Some(TypedValue::bool(true)), true, None),
                valid_dimmer(),
                invalid_unused(),
            ],
        );
        assert!(matches!(
            engine.simulate_input(missing_age),
            Err(SimulationError::MissingAge(endpoint)) if endpoint == name("wall_switch")
        ));

        let unexpected_age = simulation_scenario(
            true,
            None,
            vec![
                valid_wall(),
                valid_dimmer(),
                simulation_input("unused_input", None, false, Some(2)),
            ],
        );
        assert!(matches!(
            engine.simulate_input(unexpected_age),
            Err(SimulationError::UnexpectedAge(endpoint)) if endpoint == name("unused_input")
        ));

        let wrong_dpt = simulation_scenario(
            true,
            None,
            vec![
                simulation_input(
                    "wall_switch",
                    Some(TypedValue::percent(20).unwrap()),
                    true,
                    Some(0),
                ),
                valid_dimmer(),
                invalid_unused(),
            ],
        );
        assert!(matches!(
            engine.simulate_input(wrong_dpt),
            Err(SimulationError::DptMismatch { endpoint, .. }) if endpoint == name("wall_switch")
        ));
    }

    #[test]
    fn simulation_rejects_invalid_trigger_contract() {
        let engine = Engine::new(config());
        let complete_inputs = |wall_value, wall_age| {
            vec![
                simulation_input(
                    "wall_switch",
                    Some(TypedValue::bool(wall_value)),
                    true,
                    wall_age,
                ),
                simulation_input(
                    "dimmer_level",
                    Some(TypedValue::percent(10).unwrap()),
                    true,
                    Some(1),
                ),
                simulation_input("unused_input", None, false, None),
            ]
        };

        let mut trigger_unknown = simulation_scenario(true, None, complete_inputs(true, Some(0)));
        trigger_unknown.trigger.endpoint = name("not_configured");
        assert!(matches!(
            engine.simulate_input(trigger_unknown),
            Err(SimulationError::UnknownEndpoint(endpoint)) if endpoint == name("not_configured")
        ));

        let mut trigger_value = simulation_scenario(true, None, complete_inputs(false, Some(0)));
        trigger_value.trigger.value = TypedValue::bool(true);
        assert!(matches!(
            engine.simulate_input(trigger_value),
            Err(SimulationError::TriggerValueMismatch { endpoint, .. }) if endpoint == name("wall_switch")
        ));

        let trigger_age = simulation_scenario(true, None, complete_inputs(true, Some(9)));
        assert!(matches!(
            engine.simulate_input(trigger_age),
            Err(SimulationError::TriggerAgeMismatch { endpoint, actual: Some(9) }) if endpoint == name("wall_switch")
        ));

        let mut previous_dpt = simulation_scenario(true, None, complete_inputs(true, Some(0)));
        previous_dpt.trigger.previous = Some(TypedValue::percent(5).unwrap());
        assert!(matches!(
            engine.simulate_input(previous_dpt),
            Err(SimulationError::DptMismatch { endpoint, .. }) if endpoint == name("wall_switch")
        ));

        let invalid_trigger = simulation_scenario(
            true,
            None,
            vec![
                simulation_input("wall_switch", None, false, None),
                simulation_input(
                    "dimmer_level",
                    Some(TypedValue::percent(10).unwrap()),
                    true,
                    Some(1),
                ),
                simulation_input("unused_input", None, false, None),
            ],
        );
        assert!(matches!(
            engine.simulate_input(invalid_trigger),
            Err(SimulationError::MissingValue(endpoint)) if endpoint == name("wall_switch")
        ));
    }

    #[test]
    fn contained_lua_failures_are_returned_as_normal_simulation_executions() {
        let engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) error('simulated failure') end",
        ));
        let execution = engine
            .simulate_input(simulation_scenario(
                true,
                None,
                vec![simulation_input(
                    "wall_switch",
                    Some(TypedValue::bool(true)),
                    true,
                    Some(0),
                )],
            ))
            .unwrap();
        assert!(matches!(execution.outcome, Err(LogicError::Runtime { .. })));
    }

    #[test]
    fn equivalent_live_and_simulated_snapshots_produce_equivalent_effects() {
        let logic = "function handle(event, input)\n  if event.rising and input.enabled == true then return { outputs = { test_light = true } } end\nend";
        let endpoints = vec![
            endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
            endpoint("enabled", EndpointDirection::Input, Dpt::BOOL),
            endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
        ];
        let mut live = Engine::new(EngineConfig::new(endpoints.clone(), logic));
        live.observe_input(
            InputObservation::new(name("wall_switch"), TypedValue::bool(false)),
            at(10),
        )
        .unwrap();
        live.observe_input(
            InputObservation::new(name("enabled"), TypedValue::bool(true)),
            at(10),
        )
        .unwrap();
        let live_execution = live.process_input(trigger(true), at(20)).unwrap();

        let simulated = Engine::new(EngineConfig::new(endpoints, logic))
            .simulate_input(simulation_scenario(
                true,
                Some(false),
                vec![
                    simulation_input("wall_switch", Some(TypedValue::bool(true)), true, Some(0)),
                    simulation_input("enabled", Some(TypedValue::bool(true)), true, Some(10)),
                ],
            ))
            .unwrap();

        assert_eq!(live_execution.trigger, simulated.trigger);
        assert_eq!(live_execution.outcome, simulated.outcome);
    }

    #[test]
    fn transition_metadata_covers_first_rising_falling_and_repeated_values() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) return nil end",
        ));
        let first = run(&mut engine, true, 1);
        assert_eq!(first.trigger.previous, None);
        assert!(!first.trigger.changed);
        assert!(!first.trigger.rising);
        assert!(!first.trigger.falling);
        let repeated = run(&mut engine, true, 2);
        assert_eq!(repeated.trigger.previous, Some(TypedValue::bool(true)));
        assert!(!repeated.trigger.changed);
        assert!(!repeated.trigger.rising);
        assert!(!repeated.trigger.falling);
        let falling = run(&mut engine, false, 3);
        assert_eq!(falling.trigger.previous, Some(TypedValue::bool(true)));
        assert!(falling.trigger.changed);
        assert!(!falling.trigger.rising);
        assert!(falling.trigger.falling);
    }

    #[test]
    fn percentage_changes_set_changed_without_boolean_edges() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("level", EndpointDirection::Input, Dpt::PERCENT),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) return nil end",
        ));
        let event = |value| InputEvent::new(name("level"), TypedValue::percent(value).unwrap());
        let first = engine.process_input(event(10), MonotonicMs(1)).unwrap();
        assert!(!first.trigger.changed);
        let changed = engine.process_input(event(20), MonotonicMs(2)).unwrap();
        assert!(changed.trigger.changed);
        assert!(!changed.trigger.rising);
        assert!(!changed.trigger.falling);
        let same = engine.process_input(event(20), MonotonicMs(3)).unwrap();
        assert!(!same.trigger.changed);
    }

    #[test]
    fn passive_observations_establish_baseline_and_refresh_age() {
        let mut engine = Engine::new(config());
        engine
            .observe_input(
                InputObservation::new(name("dimmer_level"), TypedValue::percent(42).unwrap()),
                MonotonicMs(10),
            )
            .unwrap();
        let first = run(&mut engine, true, 20);
        assert_eq!(first.trigger.previous, None);
        assert_eq!(first.inputs[1].age_ms, Some(10));
        engine
            .observe_input(
                InputObservation::new(name("dimmer_level"), TypedValue::percent(42).unwrap()),
                MonotonicMs(30),
            )
            .unwrap();
        let refreshed = run(&mut engine, false, 35);
        assert_eq!(refreshed.inputs[1].age_ms, Some(5));
        assert!(effects(&refreshed).is_empty());
    }

    #[test]
    fn complete_snapshot_is_ordered_and_unknown_inputs_are_invalid() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("enabled", EndpointDirection::Input, Dpt::BOOL),
                endpoint("level", EndpointDirection::Input, Dpt::PERCENT),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input, meta)\n  return { outputs = { test_light = meta.enabled.valid == false and input.wall_switch == true and meta.wall_switch.age_ms == 0 } }\nend",
        ));
        engine
            .observe_input(
                InputObservation::new(name("level"), TypedValue::percent(9).unwrap()),
                MonotonicMs(100),
            )
            .unwrap();
        let execution = run(&mut engine, true, 150);
        assert_eq!(execution.inputs.len(), 3);
        assert_eq!(execution.inputs[0].endpoint, name("wall_switch"));
        assert_eq!(execution.inputs[0].age_ms, Some(0));
        assert!(execution.inputs[0].valid);
        assert_eq!(execution.inputs[1].value, None);
        assert!(!execution.inputs[1].valid);
        assert_eq!(execution.inputs[1].age_ms, None);
        assert_eq!(execution.inputs[2].age_ms, Some(50));
        assert_eq!(effects(&execution).len(), 1);
    }

    #[test]
    fn third_lua_argument_exposes_metadata_and_two_argument_scripts_work() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("enabled", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input, meta) return { outputs = { test_light = meta.enabled.valid and meta.enabled.age_ms == 7 and event.previous == nil } } end",
        ));
        engine
            .observe_input(
                InputObservation::new(name("enabled"), TypedValue::bool(true)),
                MonotonicMs(10),
            )
            .unwrap();
        assert_eq!(effects(&run(&mut engine, true, 17)).len(), 1);
        engine
            .replace_source(
                "function handle(event, input) return { outputs = { test_light = event.value } } end",
            )
            .unwrap();
        assert_eq!(effects(&run(&mut engine, false, 18)).len(), 1);
    }

    #[test]
    fn zero_effect_success_and_contained_lua_failure_keep_full_execution() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) return nil end",
        ));
        let success = run(&mut engine, true, 1);
        assert_eq!(
            success.outcome,
            Ok(Transition {
                state: BTreeMap::new(),
                outputs: Vec::new(),
                timers: Vec::new(),
            })
        );
        assert_eq!(success.inputs[0].value, Some(TypedValue::bool(true)));
        engine
            .replace_source("function handle(event, input) error('boom') end")
            .unwrap();
        let failed = run(&mut engine, false, 2);
        assert!(matches!(failed.outcome, Err(LogicError::Runtime { .. })));
        assert_eq!(failed.trigger.previous, Some(TypedValue::bool(true)));
        assert!(failed.inputs[0].valid);
    }

    #[test]
    fn strict_return_conversion_is_all_or_nothing() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) return { outputs = { test_light = true, nope = false } } end",
        ));
        assert!(matches!(
            run(&mut engine, true, 1).outcome,
            Err(LogicError::InvalidResult { .. })
        ));
        assert!(engine.snapshot().known_inputs.is_empty() == false);
        engine
            .replace_source(
                "function handle(event, input) return { outputs = { test_light = true } } end",
            )
            .unwrap();
        assert_eq!(effects(&run(&mut engine, false, 2)).len(), 1);
    }

    #[test]
    fn invalid_host_events_and_time_reversal_do_not_change_state() {
        let mut engine = Engine::new(config());
        assert!(matches!(
            engine.process_input(
                InputEvent::new(name("test_light"), TypedValue::bool(true)),
                MonotonicMs(1),
            ),
            Err(EventError::EndpointNotInput { .. })
        ));
        assert!(engine.snapshot().known_inputs.is_empty());
        engine
            .observe_input(
                InputObservation::new(name("wall_switch"), TypedValue::bool(true)),
                MonotonicMs(10),
            )
            .unwrap();
        assert!(matches!(
            engine.observe_input(
                InputObservation::new(name("wall_switch"), TypedValue::bool(false)),
                MonotonicMs(9),
            ),
            Err(EventError::TimeWentBackwards { .. })
        ));
        assert!(matches!(
            engine.process_input(trigger(false), MonotonicMs(8)),
            Err(EventError::TimeWentBackwards { .. })
        ));
        let execution = run(&mut engine, false, 11);
        assert_eq!(execution.trigger.previous, Some(TypedValue::bool(true)));
    }
