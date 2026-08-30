
/// The site used by legacy MonotonicMs-only execution paths: UTC time zone
/// and no coordinates. Combined with `None` as the wall-clock instant this
/// yields the unavailable time-context sentinel.

pub(crate) fn execute_logic(
    endpoints: &[Endpoint],
    program: &LogicProgram,
    snapshots: &[InputSnapshot],
    trigger: &Trigger,
    state: &TransientState,
    pending_timers: &BTreeMap<TimerName, PendingTimer>,
    now: MonotonicMs,
    time_context: &TimeContext,
    limits: &RuntimeLimits,
    budget_probe: Option<std::sync::Arc<dyn BudgetProbe>>,
) -> Result<Transition, LogicError> {
    // Validate size even though an active program was previously checked. It
    // keeps this boundary correct if a LogicProgram is constructed directly.
    check_source_size_with_limits(&program.source, limits)?;
    let lua = new_lua(limits).map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let environment =
        restricted_environment(&lua).map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    install_instruction_hook(&lua, limits, budget_probe);

    let chunk = lua
        .load(program.source.as_str())
        .set_name("logic.source")
        .set_environment(environment.clone())
        .into_function()
        .map_err(|error| map_lua_error(error, LuaPhase::Syntax))?;
    chunk
        .call::<()>(())
        .map_err(|error| map_lua_error(error, LuaPhase::Load))?;
    let handle = match environment
        .get::<LuaValue>("handle")
        .map_err(|error| map_lua_error(error, LuaPhase::Load))?
    {
        LuaValue::Function(function) => function,
        value => {
            return Err(LogicError::Load {
                message: format!(
                    "logic.source must define callable handle, got {}",
                    value.type_name()
                ),
                line: None,
            });
        }
    };

    let event_backing = lua
        .create_table()
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    event_backing
        .set("type", "input")
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    match trigger {
        Trigger::Input(trigger) => {
            event_backing
                .set("input", trigger.endpoint.as_str())
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set(
                    "value",
                    typed_value_to_lua(trigger.value),
                )
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set(
                    "previous",
                    trigger
                        .previous
                        .map(typed_value_to_lua)
                        .unwrap_or(LuaValue::Nil),
                )
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("changed", trigger.changed)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("rising", trigger.rising)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("falling", trigger.falling)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        }
        Trigger::Timer(trigger) => {
            event_backing
                .set("type", "timer")
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("timer", trigger.name.as_str())
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("scheduled_at", trigger.scheduled_at.0)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("due_at", trigger.due_at.0)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("fired_at", trigger.fired_at.0)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        }
        Trigger::Schedule(trigger) => {
            event_backing
                .set("type", "schedule")
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("block_id", trigger.block_id.as_str())
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("schedule", trigger.name.as_str())
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("kind", trigger.kind.to_string())
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("scheduled_for_utc_ms", trigger.scheduled_for_utc_ms)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("detected_at_utc_ms", trigger.detected_at_utc_ms)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("coalesced_count", trigger.coalesced_count as i64)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("structural_revision", trigger.structural_revision as i64)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        }
    }

    let input_backing = lua
        .create_table()
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let meta_backing = lua
        .create_table()
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    for snapshot in snapshots {
        if let Some(value) = snapshot.value {
            input_backing
                .set(
                    snapshot.endpoint.as_str(),
                    typed_value_to_lua(value),
                )
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        }
        let metadata_backing = lua
            .create_table()
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        metadata_backing
            .set("valid", snapshot.valid)
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        if let Some(age_ms) = snapshot.age_ms {
            metadata_backing
                .set("age_ms", age_ms)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        }
        let metadata = readonly_proxy(&lua, metadata_backing)
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        meta_backing
            .set(snapshot.endpoint.as_str(), metadata)
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    }

    let state_backing = lua
        .create_table()
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    for (key, value) in state {
        state_backing
            .set(
                key.as_str(),
                state_value_to_lua(&lua, value)
                    .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?,
            )
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    }
    let event_table = readonly_proxy(&lua, event_backing)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let input_table = readonly_proxy(&lua, input_backing)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let meta_table = readonly_proxy(&lua, meta_backing)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let state_table = readonly_proxy(&lua, state_backing)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let ctx_backing = lua
        .create_table()
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let now_userdata = lua
        .create_userdata(time_context.now.clone())
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    ctx_backing
        .set("now", now_userdata)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let sun_backing = lua
        .create_table()
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let dawn_userdata = lua
        .create_userdata(time_context.sun.dawn.clone())
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let sunrise_userdata = lua
        .create_userdata(time_context.sun.sunrise.clone())
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let sunset_userdata = lua
        .create_userdata(time_context.sun.sunset.clone())
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let dusk_userdata = lua
        .create_userdata(time_context.sun.dusk.clone())
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    sun_backing
        .set("dawn", dawn_userdata)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    sun_backing
        .set("sunrise", sunrise_userdata)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    sun_backing
        .set("sunset", sunset_userdata)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    sun_backing
        .set("dusk", dusk_userdata)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    if let Some(elevation_degrees) = time_context.sun.elevation_degrees {
        sun_backing
            .set("elevation", elevation_degrees)
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        // Keep the implementation-era spelling as a read-only compatibility
        // alias while `elevation` is the public M9 contract.
        sun_backing
            .set("elevation_degrees", elevation_degrees)
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    }
    if let Some(azimuth_degrees) = time_context.sun.azimuth_degrees {
        sun_backing
            .set("azimuth", azimuth_degrees)
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        // See the compatibility note for `elevation` above.
        sun_backing
            .set("azimuth_degrees", azimuth_degrees)
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    }
    let sun_table = readonly_proxy(&lua, sun_backing)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    ctx_backing
        .set("sun", sun_table)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let ctx_table = readonly_proxy(&lua, ctx_backing)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;

    let returned: MultiValue = handle
        .call((event_table, input_table, meta_table, state_table, ctx_table))
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let mut returned = returned.into_iter();
    let result = returned.next().unwrap_or(LuaValue::Nil);
    if returned.next().is_some() {
        return Err(LogicError::InvalidResult {
            message: "handle must return nil or one result table".to_owned(),
            line: None,
        });
    }
    convert_result(endpoints, result, state, pending_timers, now, limits)
}
