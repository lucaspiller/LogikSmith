fn id(value: &str) -> BlockId {
    BlockId::new(value).expect("valid block id")
}

fn source() -> &'static str {
    "function handle(event, inputs, meta, state, ctx) return nil end"
}

#[test]
fn built_in_profiles_expose_embedded_budgets() {
    let desktop = RuntimeProfile::Desktop.limits();
    let embedded = RuntimeProfile::EmbeddedBaseline.limits();
    assert_eq!(desktop.max_logic_blocks, 64);
    assert_eq!(embedded.max_logic_blocks, 32);
    assert_eq!(embedded.max_endpoints_per_block, 32);
    assert_eq!(embedded.max_logic_source_bytes_total, 128 * 1024);
    assert_eq!(embedded.logic_handler_time_budget_ms, Some(3));
    assert_eq!(embedded.signal_cascade_time_budget_ms, Some(4));
    assert_eq!(embedded.openknx_loop_warning_threshold_ms, Some(7));
}

#[test]
fn embedded_profile_rejects_more_than_32_endpoints() {
    let endpoints = (0..33)
        .map(|index| {
            Endpoint::new(
                EndpointName::new(format!("input{index}")).expect("valid endpoint"),
                EndpointDirection::Input,
                Dpt::BOOL,
            )
        })
        .collect();
    let config = RuntimeConfig::new(vec![BlockConfig::new(
        id("block"),
        true,
        endpoints,
        source(),
    )]);
    let error = config
        .validate_with_limits(&RuntimeProfile::EmbeddedBaseline.limits())
        .expect_err("endpoint limit should be enforced");
    assert!(error.to_string().contains("maximum is 32"));
}

#[test]
fn embedded_profile_accepts_thirty_two_blocks_and_rejects_thirty_three() {
    let block = |index: usize| {
        BlockConfig::new(
            id(&format!("block{index}")),
            true,
            vec![Endpoint::new(
                EndpointName::new("input").expect("valid endpoint"),
                EndpointDirection::Input,
                Dpt::BOOL,
            )],
            source(),
        )
    };
    let accepted = RuntimeConfig::new((0..32).map(block).collect());
    Runtime::try_new_with_profile(accepted, RuntimeProfile::EmbeddedBaseline)
        .expect("embedded baseline accepts its block ceiling");
    let rejected = RuntimeConfig::new((0..33).map(block).collect());
    let error = Runtime::try_new_with_profile(rejected, RuntimeProfile::EmbeddedBaseline)
        .expect_err("embedded baseline rejects one block over its ceiling");
    assert!(error.to_string().contains("maximum is 32"));
}

#[test]
fn embedded_baseline_budget_harness_reports_bounded_usage() {
    let blocks = (0..32)
        .map(|index| {
            let mut endpoints = Vec::with_capacity(32);
            for endpoint_index in 0..16 {
                endpoints.push(Endpoint::new(
                    EndpointName::new(format!("input{endpoint_index}"))
                        .expect("valid input endpoint"),
                    EndpointDirection::Input,
                    Dpt::BOOL,
                ));
            }
            for endpoint_index in 0..16 {
                endpoints.push(Endpoint::new(
                    EndpointName::new(format!("output{endpoint_index}"))
                        .expect("valid output endpoint"),
                    EndpointDirection::Output,
                    Dpt::BOOL,
                ));
            }
            let source = format!(
                "function handle(event) return nil end\n-- {}\n",
                "x".repeat(4_000)
            );
            BlockConfig::new(
                id(&format!("block{index}")),
                true,
                endpoints,
                source,
            )
        })
        .collect();
    let runtime = Runtime::try_new_with_profile(
        RuntimeConfig::new(blocks),
        RuntimeProfile::EmbeddedBaseline,
    )
    .expect("maximum embedded planning document remains valid");
    let limits = RuntimeProfile::EmbeddedBaseline.limits();
    let usage = runtime.usage();
    assert_eq!(usage.logic_blocks, limits.max_logic_blocks);
    assert!(usage.logic_source_bytes <= limits.max_logic_source_bytes_total);
    assert!(usage.state_bytes <= limits.max_state_bytes_total);
    assert!(usage.pending_timers <= limits.max_pending_timers_total);
}

#[test]
fn handler_time_budget_uses_host_probe() {
    struct Probe;
    impl BudgetProbe for Probe {
        fn elapsed_ms(&self) -> u64 {
            4
        }
    }
    let config = RuntimeConfig::new(vec![BlockConfig::new(
        id("block"),
        true,
        vec![Endpoint::new(
            EndpointName::new("input").expect("valid endpoint"),
            EndpointDirection::Input,
            Dpt::BOOL,
        )],
        source(),
    )]);
    let runtime = Runtime::try_new_with_profile(config, RuntimeProfile::EmbeddedBaseline)
        .expect("valid embedded runtime");
    assert!(matches!(
        runtime.check_cascade_budget(&Probe),
        Err(RuntimeEventError::CascadeTimeLimit {
            elapsed_ms: 4,
            maximum_ms: 4
        })
    ));
}

#[test]
fn five_live_failures_suspend_only_the_block() {
    let config = RuntimeConfig::new(vec![BlockConfig::new(
        id("block"),
        true,
        vec![Endpoint::new(
            EndpointName::new("input").expect("valid endpoint"),
            EndpointDirection::Input,
            Dpt::BOOL,
        )],
        "function handle(event) error('boom') end",
    )]);
    let mut runtime = Runtime::try_new(config).expect("valid runtime");
    let endpoint = EndpointName::new("input").expect("valid endpoint");
    for now in 0..5 {
        let execution = runtime
            .process_input(&id("block"), InputEvent::new(endpoint.clone(), TypedValue::bool(true)), MonotonicMs(now))
            .expect("event should be contained");
        assert!(execution.is_some());
    }
    let block = runtime.snapshot().blocks.into_iter().next().expect("block");
    assert_eq!(block.health, BlockHealth::SuspendedScriptFailures);
    assert_eq!(block.consecutive_failures, 5);
    assert!(runtime
        .process_input(
            &id("block"),
            InputEvent::new(endpoint.clone(), TypedValue::bool(false)),
            MonotonicMs(6),
        )
        .expect("suspended input is observed")
        .is_none());
    runtime.resume_block(&id("block")).expect("resume block");
    assert_eq!(runtime.snapshot().blocks[0].health, BlockHealth::Active);
}

#[test]
fn live_rate_limit_suspends_without_entering_lua() {
    let config = RuntimeConfig::new(vec![BlockConfig::new(
        id("block"),
        true,
        vec![Endpoint::new(
            EndpointName::new("input").expect("valid endpoint"),
            EndpointDirection::Input,
            Dpt::BOOL,
        )],
        source(),
    )]);
    let mut limits = RuntimeLimits::desktop();
    limits.max_live_executions_per_block_per_second = 2;
    let mut runtime = Runtime::try_new_with_limits(config, limits).expect("valid runtime");
    let endpoint = EndpointName::new("input").expect("valid endpoint");
    for now in 0..2 {
        assert!(runtime
            .process_input(
                &id("block"),
                InputEvent::new(endpoint.clone(), TypedValue::bool(true)),
                MonotonicMs(now),
            )
            .expect("event should execute")
            .is_some());
    }
    assert!(runtime
        .process_input(
            &id("block"),
            InputEvent::new(endpoint, TypedValue::bool(false)),
            MonotonicMs(2),
        )
        .expect("rate-rejected input is observed")
        .is_none());
    assert_eq!(runtime.snapshot().blocks[0].health, BlockHealth::SuspendedEventRate);
}

#[cfg(not(feature = "timezones"))]
#[test]
fn timezone_feature_is_utc_only() {
    assert!(TimeZoneId::new("UTC").is_ok());
    assert!(TimeZoneId::new("Europe/Berlin").is_err());
}

#[cfg(not(feature = "astronomy"))]
#[test]
fn astronomy_feature_fails_closed() {
    let rule = ScheduleRule::Astronomical {
        anchor: SolarAnchor::Sunrise,
        offset_seconds: 0,
        weekdays: WeekdaySet::new(&Weekday::ALL).expect("all weekdays"),
    };
    assert!(matches!(rule.validate(), Err(ScheduleError::FeatureDisabled { .. })));
}
