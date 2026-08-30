use super::*;

fn endpoint(name: &str) -> EndpointName {
    name.parse().expect("valid endpoint name")
}
fn block_id(name: &str) -> BlockId {
    name.parse().expect("valid block id")
}

fn temperature(value: f64) -> TypedValue {
    TypedValue::temperature(value).expect("valid temperature")
}

#[test]
fn dpt_9_001_uses_canonical_signed_centi_degrees() {
    assert_eq!(Dpt::parse("9.001").unwrap(), Dpt::TEMPERATURE);
    assert!(Dpt::TEMPERATURE.is_supported());
    assert!(Dpt::TEMPERATURE.is_temperature());

    let value = temperature(-4.25);
    assert_eq!(value.value(), Value::Temperature(-425));
    assert_eq!(value.temperature_centi(), Some(-425));
    assert_eq!(value.temperature_celsius_value(), Some(-4.25));
    assert_eq!(
        TypedValue::temperature_centi_degrees(1234).unwrap(),
        temperature(12.34)
    );
}

#[test]
fn dpt_9_001_rejects_non_finite_and_more_than_two_decimal_places() {
    assert_eq!(
        TypedValue::temperature(f64::NAN),
        Err(ValueError::TemperatureNotFinite)
    );
    assert_eq!(
        TypedValue::temperature(f64::INFINITY),
        Err(ValueError::TemperatureNotFinite)
    );
    assert_eq!(
        TypedValue::temperature(12.345),
        Err(ValueError::TemperaturePrecision)
    );
    assert!(TypedValue::new(Dpt::TEMPERATURE, Value::Percent(42)).is_err());
}

#[test]
fn dpt_9_001_round_trips_through_lua_as_degrees_celsius() {
    let config = EngineConfig::new(
        vec![
            Endpoint::input(endpoint("temperature"), Dpt::TEMPERATURE),
            Endpoint::output(endpoint("adjusted"), Dpt::TEMPERATURE),
        ],
        r#"
            function handle(event, input)
                if event.type == "input" then
                    return { outputs = { adjusted = input.temperature + 1.25 } }
                end
            end
        "#,
    );
    let mut engine = Engine::new(config);
    let execution = engine
        .process_input(
            InputEvent::new(endpoint("temperature"), temperature(12.34)),
            MonotonicMs(10),
        )
        .unwrap();
    let outputs = execution.outcome.unwrap().outputs;
    assert_eq!(outputs[0].value, temperature(13.59));
}

#[test]
fn input_updates_observe_trigger_and_invalidate_have_distinct_semantics() {
    let block = BlockConfig::new(
        block_id("weather"),
        true,
        vec![
            Endpoint::input(endpoint("temperature"), Dpt::TEMPERATURE),
            Endpoint::output(endpoint("adjusted"), Dpt::TEMPERATURE),
        ],
        r#"
            function handle(event, input)
                if event.type == "input" then
                    return { outputs = { adjusted = input.temperature + 1 } }
                end
            end
        "#,
    );
    let mut runtime = Runtime::new(RuntimeConfig::new(vec![block]));
    let block_id = block_id("weather");
    let input = endpoint("temperature");

    assert!(runtime
        .process_input_update(
            &block_id,
            input.clone(),
            InputUpdate::Observe(temperature(10.5)),
            MonotonicMs(10),
        )
        .unwrap()
        .is_empty());
    let snapshot = runtime.snapshot_at(MonotonicMs(15));
    assert_eq!(snapshot.blocks[0].inputs[0].value, Some(temperature(10.5)));
    assert!(snapshot.blocks[0].inputs[0].valid);
    assert_eq!(snapshot.blocks[0].inputs[0].age_ms, Some(5));

    assert!(runtime
        .process_input_update(
            &block_id,
            input.clone(),
            InputUpdate::Invalidate,
            MonotonicMs(20),
        )
        .unwrap()
        .is_empty());
    let snapshot = runtime.snapshot_at(MonotonicMs(20));
    assert!(!snapshot.blocks[0].inputs[0].valid);
    assert_eq!(snapshot.blocks[0].inputs[0].value, None);
    assert_eq!(snapshot.blocks[0].inputs[0].age_ms, None);

    let executions = runtime
        .process_input_update(
            &block_id,
            input,
            InputUpdate::Trigger(temperature(11.25)),
            MonotonicMs(30),
        )
        .unwrap();
    assert_eq!(executions.len(), 1);
    assert_eq!(
        executions[0].execution.trigger,
        Trigger::Input(InputTrigger {
            endpoint: endpoint("temperature"),
            value: temperature(11.25),
            previous: None,
            changed: false,
            rising: false,
            falling: false,
        })
    );
    assert_eq!(
        executions[0].execution.outcome.as_ref().unwrap().outputs[0].value,
        temperature(12.25)
    );
}
