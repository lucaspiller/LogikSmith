use crate::lua::READ_ONLY_ARGUMENT_MARKER;

    fn n(value: &str) -> EndpointName {
        value.parse().unwrap()
    }
    fn t(value: &str) -> TimerName {
        value.parse().unwrap()
    }
    fn endpoint(value: &str, direction: EndpointDirection, dpt: Dpt) -> Endpoint {
        Endpoint::new(n(value), direction, dpt)
    }
    fn engine(source: &str) -> Engine {
        Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            source,
        ))
    }
    fn event(value: bool) -> InputEvent {
        InputEvent::new(n("wall_switch"), TypedValue::bool(value))
    }

    #[test]
    fn state_and_named_timers_round_trip_and_expire() {
        let mut engine = engine(
            r#"
            function handle(event, input, meta, state)
              if event.type == "input" and event.rising then
                return { state = { count = (state.count or 0) + 1 }, outputs = { test_light = true }, timers = { dim = { after = seconds(2) }, off = { after = seconds(3) } } }
              end
              if event.type == "timer" and event.timer == "off" then
                return { state = { done = true }, outputs = { test_light = false } }
              end
            end
        "#,
        );
        engine.process_input(event(false), MonotonicMs(1)).unwrap();
        let first = engine.process_input(event(true), MonotonicMs(10)).unwrap();
        let transition = first.outcome.unwrap();
        assert_eq!(transition.state["count"], StateValue::Integer(1));
        assert_eq!(transition.timers.len(), 2);
        assert_eq!(engine.pending_timers()[0].name, t("dim"));
        assert_eq!(engine.next_timer_deadline(), Some(MonotonicMs(2010)));
        assert!(
            engine
                .process_next_due_timer(MonotonicMs(2009))
                .unwrap()
                .is_none()
        );
        let _dim = engine
            .process_next_due_timer(MonotonicMs(3010))
            .unwrap()
            .unwrap();
        let timer = engine
            .process_next_due_timer(MonotonicMs(3010))
            .unwrap()
            .unwrap();
        assert!(
            matches!(timer.trigger, Trigger::Timer(TimerTrigger { ref name, scheduled_at: MonotonicMs(10), due_at: MonotonicMs(3010), fired_at: MonotonicMs(3010), .. }) if name == &t("off"))
        );
        assert_eq!(engine.state()["done"], StateValue::Bool(true));
    }

    #[test]
    fn failed_transition_rolls_back_state_outputs_and_timers() {
        let mut engine = engine(
            r#"function handle() return { state = { value = { bad = true } }, outputs = { test_light = true }, timers = { off = { after = 2 } } } end"#,
        );
        let execution = engine.process_input(event(true), MonotonicMs(1)).unwrap();
        assert!(matches!(
            execution.outcome,
            Err(LogicError::InvalidResult { .. })
        ));
        assert!(engine.state().is_empty());
        assert!(engine.pending_timers().is_empty());
    }

    #[test]
    fn read_only_views_reject_assignment_and_pairs() {
        let mut bad_engine =
            engine(r#"function handle(event, input, meta, state) event.x = 1 end"#);
        let execution = bad_engine
            .process_input(event(true), MonotonicMs(1))
            .unwrap();
        assert!(
            matches!(execution.outcome, Err(LogicError::Runtime { message, .. }) if message.contains(READ_ONLY_ARGUMENT_MARKER))
        );
        let mut engine2 = engine(
            r#"function handle(event, input, meta, state) local seen = 0 for k,v in pairs(meta) do seen = seen + 1 end local k,v = next(meta) assert(k == "wall_switch") return { state = { seen = seen } } end"#,
        );
        let execution = engine2.process_input(event(true), MonotonicMs(1)).unwrap();
        assert!(execution.outcome.is_ok(), "{:?}", execution.outcome);
    }

    #[test]
    fn duration_helpers_accept_fractional_values() {
        let mut engine = engine(
            r#"function handle() return { timers = { off = { after = seconds(1.5) } } } end"#,
        );
        engine.process_input(event(true), MonotonicMs(4)).unwrap();
        assert_eq!(engine.pending_timers()[0].due_at, MonotonicMs(1504));
    }

    #[test]
    fn timer_replacement_cancellation_and_activation_are_atomic() {
        let mut engine = engine(
            r#"function handle() return { timers = { off = { after = 100 }, dim = { after = 200 } } } end"#,
        );
        engine.process_input(event(true), MonotonicMs(10)).unwrap();
        engine.process_input(event(false), MonotonicMs(20)).unwrap();
        assert_eq!(engine.pending_timers().len(), 2);
        let cancelled = engine
            .activate_source(r#"function handle() return nil end"#)
            .unwrap();
        assert_eq!(cancelled.cancelled_timers, vec![t("dim"), t("off")]);
        assert!(engine.pending_timers().is_empty());
        let same = engine
            .activate_source(r#"function handle() return nil end"#)
            .unwrap();
        assert!(!same.changed);
    }

    #[test]
    fn timer_simulation_does_not_mutate_live_state() {
        let mut engine = engine(
            r#"function handle(event, input, meta, state) if event.type == "input" then return { state = { count = 1 }, timers = { off = { after = 10 } } } end return { state = { count = 2 } } end"#,
        );
        engine.process_input(event(true), MonotonicMs(10)).unwrap();
        let before = engine.snapshot();
        let simulation = engine
            .simulate_timer(TimerSimulationScenario {
                timer: t("off"),
                fired_at: MonotonicMs(25),
                inputs: vec![SimulationInput {
                    endpoint: n("wall_switch"),
                    value: Some(TypedValue::bool(true)),
                    valid: true,
                    age_ms: Some(15),
                }],
                state: before.state.clone(),
                pending_timers: before.pending_timers.clone(),
            })
            .unwrap();
        assert_eq!(
            simulation.state_after["count"],
            StateValue::Integer(2),
            "{:?}",
            simulation.outcome
        );
        assert_eq!(engine.snapshot(), before);
    }
