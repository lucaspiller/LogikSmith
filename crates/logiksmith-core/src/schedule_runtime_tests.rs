    #[test]
    fn sampled_schedule_execution_uses_handling_monotonic_time_for_timers() {
        let source = "function handle(event, input, meta, state, ctx) if event.type == 'schedule' then return { timers = { off = { after = seconds(5) } } } end return {} end";
        let mut runtime = Runtime::new(RuntimeConfig::with_site(
            vec![BlockConfig::with_schedules(
                id("a"),
                true,
                vec![
                    Endpoint::input("input".parse().unwrap(), Dpt::BOOL),
                    Endpoint::output("light".parse().unwrap(), Dpt::BOOL),
                ],
                source,
                vec![schedule("m", true, interval(3600, 0))],
            )],
            utc_site(),
        ));
        let baseline = utc_ms(2026, 6, 1, 10, 0, 0);
        runtime
            .initialise_schedules(sample_at(100, Some(baseline)), 3)
            .unwrap();
        let handling_sample = sample_at(1_000, Some(baseline + 3_600_000 + 500));
        let trigger = runtime.poll_schedules(handling_sample).unwrap().remove(0);
        let execution = runtime
            .process_schedule_sampled(trigger, handling_sample)
            .unwrap()
            .unwrap();
        assert_eq!(
            execution.execution.pending_timers[0].scheduled_at,
            MonotonicMs(1_000)
        );
        assert_eq!(
            execution.execution.pending_timers[0].due_at,
            MonotonicMs(6_000)
        );
        assert_eq!(runtime.last_accepted_at(), Some(MonotonicMs(1_000)));
    }

    // --- simulate_schedule ---------------------------------------------------

    #[test]
    fn simulate_schedule_validates_and_does_not_mutate() {
        let source = "function handle(event, input, meta, state, ctx) if event.type == 'schedule' then return { state = { y = ctx.now.year, name = event.schedule } } end return {} end";
        let mut runtime = Runtime::new(RuntimeConfig::with_site(
            vec![BlockConfig::with_schedules(
                id("a"),
                true,
                vec![
                    Endpoint::input("input".parse().unwrap(), Dpt::BOOL),
                    Endpoint::output("light".parse().unwrap(), Dpt::BOOL),
                ],
                source,
                vec![schedule("m", true, interval(3600, 0))],
            )],
            utc_site(),
        ));
        runtime
            .initialise_schedules(sample(utc_ms(2026, 6, 1, 10, 0, 0)), 7)
            .unwrap();
        let before = runtime.snapshot();
        let logic_revision = runtime.block(&id("a")).unwrap().active_logic_revision();
        let request = ScheduleSimulationRequest {
            block_id: id("a"),
            expected_logic_revision: logic_revision,
            expected_structural_revision: 7,
            schedule: sname("m"),
            occurrence_at_utc_ms: utc_ms(2026, 6, 1, 11, 0, 0),
        };
        let execution = runtime.simulate_schedule(request.clone()).unwrap();
        assert!(matches!(execution.execution.trigger, Trigger::Schedule(_)));
        assert_eq!(
            execution.execution.state_after["y"],
            StateValue::Integer(2026)
        );
        assert_eq!(
            execution.execution.state_after["name"],
            StateValue::String("m".to_owned())
        );
        // Simulation mutates nothing.
        assert_eq!(runtime.snapshot(), before);
        // Not an occurrence.
        let not_occurrence = ScheduleSimulationRequest {
            occurrence_at_utc_ms: utc_ms(2026, 6, 1, 11, 0, 30),
            ..request.clone()
        };
        assert_eq!(
            runtime.simulate_schedule(not_occurrence),
            Err(ScheduleSimulationError::NotOccurrence)
        );
        // A mathematically valid but historical occurrence is outside the
        // current preview window and cannot be selected for simulation.
        let historical = ScheduleSimulationRequest {
            occurrence_at_utc_ms: utc_ms(2026, 6, 1, 10, 0, 0),
            ..request.clone()
        };
        assert_eq!(
            runtime.simulate_schedule(historical),
            Err(ScheduleSimulationError::NotOccurrence)
        );
        // Stale structural revision.
        let stale_structural = ScheduleSimulationRequest {
            expected_structural_revision: 99,
            ..request.clone()
        };
        assert_eq!(
            runtime.simulate_schedule(stale_structural),
            Err(ScheduleSimulationError::StaleStructuralRevision)
        );
        // Stale logic revision.
        let stale_logic = ScheduleSimulationRequest {
            expected_logic_revision: logic_revision ^ 1,
            ..request.clone()
        };
        assert_eq!(
            runtime.simulate_schedule(stale_logic),
            Err(ScheduleSimulationError::StaleStructuralRevision)
        );
        // Unknown schedule.
        let unknown = ScheduleSimulationRequest {
            schedule: sname("nope"),
            ..request.clone()
        };
        assert_eq!(
            runtime.simulate_schedule(unknown),
            Err(ScheduleSimulationError::UnknownSchedule)
        );
        // Unknown block.
        let unknown_block = ScheduleSimulationRequest {
            block_id: id("nope"),
            ..request.clone()
        };
        assert_eq!(
            runtime.simulate_schedule(unknown_block),
            Err(ScheduleSimulationError::UnknownSchedule)
        );
    }

    // --- configuration bounds ------------------------------------------------

    #[test]
    fn block_config_rejects_more_than_32_schedules_and_duplicates() {
        let many: Vec<BlockSchedule> = (0..=MAX_SCHEDULES_PER_BLOCK)
            .map(|index| schedule(&format!("s{index}"), true, interval(60, 0)))
            .collect();
        assert!(matches!(
            block_config("a", true, many).validate(),
            Err(BlockConfigError::TooManySchedules {
                actual: 33,
                maximum: 32
            })
        ));
        let duplicated = vec![
            schedule("same", true, interval(60, 0)),
            schedule("same", true, interval(60, 0)),
        ];
        assert!(matches!(
            block_config("a", true, duplicated).validate(),
            Err(BlockConfigError::DuplicateSchedule(_))
        ));
        // The boundary itself is accepted.
        let boundary: Vec<BlockSchedule> = (0..MAX_SCHEDULES_PER_BLOCK)
            .map(|index| schedule(&format!("s{index}"), true, interval(60, 0)))
            .collect();
        assert!(block_config("a", true, boundary).validate().is_ok());
    }

    #[test]
    fn schedule_rule_bounds_are_validated() {
        let cases: Vec<(ScheduleRule, ScheduleError)> = vec![
            (
                interval(30, 0),
                ScheduleError::InvalidInterval { every_seconds: 30 },
            ),
            (
                interval(604_801, 0),
                ScheduleError::InvalidInterval {
                    every_seconds: 604_801,
                },
            ),
            (
                interval(60, 60),
                ScheduleError::InvalidIntervalOffset {
                    offset_seconds: 60,
                    every_seconds: 60,
                },
            ),
            (
                astro(SolarAnchor::Sunrise, 90_000, None, None, &Weekday::ALL),
                ScheduleError::InvalidAstronomicalOffset {
                    offset_seconds: 90_000,
                },
            ),
            (
                fixed(
                    LocalTime {
                        hour: 24,
                        minute: 0,
                        second: 0,
                    },
                    &Weekday::ALL,
                ),
                ScheduleError::InvalidLocalTime(LocalTime {
                    hour: 24,
                    minute: 0,
                    second: 0,
                }),
            ),
        ];
        for (rule, expected) in cases {
            assert_eq!(rule.validate(), Err(expected));
        }
        // Valid boundary rules pass.
        assert!(interval(60, 0).validate().is_ok());
        assert!(interval(604_800, 604_799).validate().is_ok());
        assert!(
            astro(SolarAnchor::Dusk, -86_400, None, None, &Weekday::ALL)
                .validate()
                .is_ok()
        );
        // An invalid rule surfaces through BlockConfig::validate.
        let config = block_config("a", true, vec![schedule("bad", true, interval(30, 0))]);
        assert!(matches!(
            config.validate(),
            Err(BlockConfigError::InvalidSchedule { .. })
        ));
    }

    #[test]
    fn interval_boundary_rules_never_panic() {
        // Defensive engine behaviour for unvalidated rules: no panic, no loop.
        let site = utc_site();
        assert_eq!(
            next_occurrence_after(&interval(0, 0), &site, 1_700_000_000_000),
            None
        );
        assert!(next_occurrence_after(&interval(60, 0), &site, 1_700_000_000_000).is_some());
        let bad_time = fixed(
            LocalTime {
                hour: 99,
                minute: 0,
                second: 0,
            },
            &Weekday::ALL,
        );
        assert_eq!(
            next_occurrence_after(&bad_time, &site, 1_700_000_000_000),
            None
        );
    }

    // --- time context --------------------------------------------------------

    #[test]
    fn capture_builds_now_and_sun_fields() {
        let context = TimeContext::capture(&utc_site(), Some(utc_ms(2026, 6, 4, 13, 45, 30)));
        assert!(context.now.available);
        assert_eq!(context.now.year, Some(2026));
        assert_eq!(context.now.month, Some(6));
        assert_eq!(context.now.day, Some(4));
        assert_eq!(context.now.hour, Some(13));
        assert_eq!(context.now.minute, Some(45));
        assert_eq!(context.now.second, Some(30));
        assert_eq!(context.now.weekday, Some(Weekday::Thursday));
        // Equator: every solar event exists every day.
        assert!(context.sun.dawn.available);
        assert!(context.sun.sunrise.available);
        assert!(context.sun.sunset.available);
        assert!(context.sun.dusk.available);
        assert!(context.sun.elevation_degrees.is_some());
        assert!(context.sun.azimuth_degrees.is_some());
        // Events land on the same local date as `now`.
        assert_eq!(context.sun.sunrise.weekday, Some(Weekday::Thursday));
        // The sun context is a different instant than `now`.
        assert_ne!(context.now.instant, context.sun.sunrise.instant);
    }

    #[test]
    fn capture_without_clock_or_coordinates_returns_unavailable() {
        let no_clock = TimeContext::capture(&utc_site(), None);
        assert!(!no_clock.now.available);
        assert_eq!(no_clock.now.year, None);
        assert_eq!(no_clock.now.month, None);
        assert_eq!(no_clock.now.day, None);
        assert_eq!(no_clock.now.hour, None);
        assert_eq!(no_clock.now.minute, None);
        assert_eq!(no_clock.now.second, None);
        assert_eq!(no_clock.now.weekday, None);
        assert!(!no_clock.sun.dawn.available);
        assert!(!no_clock.sun.sunrise.available);
        assert!(!no_clock.sun.sunset.available);
        assert!(!no_clock.sun.dusk.available);
        assert_eq!(no_clock.sun.elevation_degrees, None);
        assert_eq!(no_clock.sun.azimuth_degrees, None);
        // Clock present but no coordinates: `now` available, sun unavailable.
        let no_coords = SiteTimeConfig {
            timezone: TimeZoneId::utc(),
            coordinates: None,
        };
        let context = TimeContext::capture(&no_coords, Some(1_700_000_000_000));
        assert!(context.now.available);
        assert!(!context.sun.sunrise.available);
        assert_eq!(context.sun.elevation_degrees, None);
    }

    #[test]
    fn time_context_capture_is_stable() {
        let instant = utc_ms(2026, 6, 4, 13, 45, 30);
        assert_eq!(
            TimeContext::capture(&utc_site(), Some(instant)),
            TimeContext::capture(&utc_site(), Some(instant))
        );
    }

    // --- Lua ctx integration -------------------------------------------------

    fn ctx_engine(source: &str) -> Engine {
        Engine::new(EngineConfig::new(
            vec![Endpoint::input("wall_switch".parse().unwrap(), Dpt::BOOL)],
            source,
        ))
    }

    #[test]
    fn ctx_exposes_fields_and_comparisons() {
        let mut engine = ctx_engine(
            r#"
            function handle(event, input, meta, state, ctx)
                return { state = {
                    year = ctx.now.year,
                    month = ctx.now.month,
                    day = ctx.now.day,
                    hour = ctx.now.hour,
                    minute = ctx.now.minute,
                    second = ctx.now.second,
                    weekday = ctx.now.weekday,
                    eq = ctx.now == ctx.now,
                    lt = ctx.now < ctx.now,
                    le = ctx.now <= ctx.now,
                    ne = ctx.now ~= ctx.sun.sunrise,
                    now_le_sunset = ctx.now <= ctx.sun.sunset,
                    sun_elevation = ctx.sun.elevation,
                    sun_azimuth = ctx.sun.azimuth,
                    dawn_weekday = ctx.sun.dawn.weekday,
                }}
            end
            "#,
        );
        let execution = engine
            .process_input_sampled(
                InputEvent::new("wall_switch".parse().unwrap(), TypedValue::bool(true)),
                sample(utc_ms(2026, 6, 4, 13, 45, 30)),
                &utc_site(),
            )
            .unwrap();
        assert!(
            matches!(execution.outcome, Ok(_)),
            "{:?}",
            execution.outcome
        );
        let state = &execution.state_after;
        assert_eq!(state["year"], StateValue::Integer(2026));
        assert_eq!(state["month"], StateValue::Integer(6));
        assert_eq!(state["day"], StateValue::Integer(4));
        assert_eq!(state["hour"], StateValue::Integer(13));
        assert_eq!(state["minute"], StateValue::Integer(45));
        assert_eq!(state["second"], StateValue::Integer(30));
        assert_eq!(state["weekday"], StateValue::String("Thursday".to_owned()));
        assert_eq!(state["eq"], StateValue::Bool(true));
        assert_eq!(state["lt"], StateValue::Bool(false));
        assert_eq!(state["le"], StateValue::Bool(true));
        assert_eq!(state["ne"], StateValue::Bool(true));
        // 13:45 is before sunset at the equator.
        assert_eq!(state["now_le_sunset"], StateValue::Bool(true));
        assert!(matches!(
            state["sun_elevation"],
            StateValue::Number(value) if value.is_finite()
        ));
        assert!(matches!(
            state["sun_azimuth"],
            StateValue::Number(value) if value.is_finite()
        ));
        assert_eq!(
            state["dawn_weekday"],
            StateValue::String("Thursday".to_owned())
        );
    }

    #[test]
    fn ctx_unavailable_sentinels_compare_false_and_fields_nil() {
        let mut engine = ctx_engine(
            r#"
            function handle(event, input, meta, state, ctx)
                return { state = {
                    eq = ctx.now == ctx.sun.dawn,
                    lt = ctx.now < ctx.now,
                    le = ctx.now <= ctx.now,
                    year_nil = (ctx.now.year == nil),
                    month_nil = (ctx.now.month == nil),
                    weekday_nil = (ctx.now.weekday == nil),
                    sun_eq = ctx.sun.dawn == ctx.sun.sunrise,
                    elev_nil = (ctx.sun.elevation == nil),
                }}
            end
            "#,
        );
        // Legacy MonotonicMs path: no wall clock -> unavailable sentinel.
        let execution = engine
            .process_input(
                InputEvent::new("wall_switch".parse().unwrap(), TypedValue::bool(true)),
                MonotonicMs(0),
            )
            .unwrap();
        assert!(
            matches!(execution.outcome, Ok(_)),
            "{:?}",
            execution.outcome
        );
        assert!(!execution.time_context.now.available);
        let state = &execution.state_after;
        assert_eq!(state["eq"], StateValue::Bool(false));
        assert_eq!(state["lt"], StateValue::Bool(false));
        assert_eq!(state["le"], StateValue::Bool(false));
        assert_eq!(state["year_nil"], StateValue::Bool(true));
        assert_eq!(state["month_nil"], StateValue::Bool(true));
        assert_eq!(state["weekday_nil"], StateValue::Bool(true));
        assert_eq!(state["sun_eq"], StateValue::Bool(false));
        assert_eq!(state["elev_nil"], StateValue::Bool(true));
    }

    #[test]
    fn ctx_is_read_only() {
        // Each mutation aborts the handler; the execution outcome carries the
        // contained Lua error.
        let scripts = [
            "function handle(event, input, meta, state, ctx) ctx.now = 1 end",
            "function handle(event, input, meta, state, ctx) ctx.sun.sunrise = 1 end",
            "function handle(event, input, meta, state, ctx) ctx.now.year = 1 end",
            "function handle(event, input, meta, state, ctx) weekdays.Monday = 'x' end",
        ];
        for source in scripts {
            let mut engine = ctx_engine(source);
            let execution = engine
                .process_input(
                    InputEvent::new("wall_switch".parse().unwrap(), TypedValue::bool(true)),
                    MonotonicMs(0),
                )
                .unwrap();
            assert!(matches!(execution.outcome, Err(_)), "{source}");
        }
    }

    #[test]
    fn weekdays_table_exposes_full_names() {
        let mut engine = ctx_engine(
            r#"
            function handle(event, input, meta, state, ctx)
                return { state = {
                    first = weekdays[1],
                    seventh = weekdays[7],
                    monday = weekdays.Monday,
                    sunday = weekdays.Sunday,
                }}
            end
            "#,
        );
        let execution = engine
            .process_input(
                InputEvent::new("wall_switch".parse().unwrap(), TypedValue::bool(true)),
                MonotonicMs(0),
            )
            .unwrap();
        assert!(
            matches!(execution.outcome, Ok(_)),
            "{:?}",
            execution.outcome
        );
        let state = &execution.state_after;
        assert_eq!(state["first"], StateValue::String("Monday".to_owned()));
        assert_eq!(state["seventh"], StateValue::String("Sunday".to_owned()));
        assert_eq!(state["monday"], StateValue::String("Monday".to_owned()));
        assert_eq!(state["sunday"], StateValue::String("Sunday".to_owned()));
    }

    #[test]
    fn older_arity_handlers_still_work() {
        let sources = [
            "function handle() return {} end",
            "function handle(event) return {} end",
            "function handle(event, input) return {} end",
            "function handle(event, input, meta) return {} end",
            "function handle(event, input, meta, state) return {} end",
        ];
        for source in sources {
            let mut engine = ctx_engine(source);
            let execution = engine
                .process_input(
                    InputEvent::new("wall_switch".parse().unwrap(), TypedValue::bool(true)),
                    MonotonicMs(0),
                )
                .unwrap();
            assert!(matches!(execution.outcome, Ok(_)), "{source}");
        }
    }

    #[test]
    fn sampled_execution_is_stable_across_runs() {
        let mut engine = ctx_engine(
            r#"
            function handle(event, input, meta, state, ctx)
                return { state = { y = ctx.now.year, d = ctx.now.day } }
            end
            "#,
        );
        let event = InputEvent::new("wall_switch".parse().unwrap(), TypedValue::bool(true));
        let sample = sample(utc_ms(2026, 6, 4, 13, 45, 30));
        let first = engine
            .process_input_sampled(event.clone(), sample, &utc_site())
            .unwrap();
        let second = engine
            .process_input_sampled(event, sample, &utc_site())
            .unwrap();
        assert_eq!(first.state_after, second.state_after);
        assert_eq!(first.time_context, second.time_context);
    }
