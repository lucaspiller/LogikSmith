use std::{cell::Cell, collections::BTreeMap, rc::Rc};
use std::sync::Arc;

use mlua::{
    Function, HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, Value as LuaValue, VmState,
};

use crate::state::validate_state_entry;
use crate::*;

enum LuaPhase {
    Syntax,
    Load,
    Runtime,
    InvalidResult,
}

fn check_source_size_with_limits(source: &str, limits: &RuntimeLimits) -> Result<(), LogicError> {
    if source.trim().is_empty() {
        return Err(LogicError::EmptySource);
    }
    if source.len() > limits.max_logic_source_bytes_per_block {
        return Err(LogicError::SourceTooLarge {
            actual: source.len(),
            maximum: limits.max_logic_source_bytes_per_block,
        });
    }
    Ok(())
}

fn new_lua(limits: &RuntimeLimits) -> Result<Lua, mlua::Error> {
    // Base functions are always initialized by Lua, but the explicit library
    // list ensures package/io/os/debug/coroutine are never loaded at all.
    let lua = Lua::new_with(
        StdLib::MATH | StdLib::STRING | StdLib::TABLE | StdLib::UTF8,
        LuaOptions::default(),
    )?;
    lua.set_memory_limit(limits.max_logic_memory_bytes)?;
    Ok(lua)
}

fn restricted_environment(lua: &Lua) -> Result<Table, mlua::Error> {
    let globals = lua.globals();
    let environment = lua.create_table()?;

    // Keep this allowlist intentionally small. In particular, `_G`, load,
    // dofile, loadfile, require, print, and collectgarbage are not copied into
    // the per-execution environment.
    const SAFE_BASE: &[&str] = &[
        "assert", "error", "ipairs", "pairs", "select", "tonumber", "tostring", "type",
    ];
    for name in SAFE_BASE {
        let value: LuaValue = globals.get(*name)?;
        environment.set(*name, value)?;
    }
    for name in ["math", "string", "table", "utf8"] {
        let value: LuaValue = globals.get(name)?;
        environment.set(name, value)?;
    }
    let builtin_next: Function = globals.get("next")?;
    let safe_next = lua.create_function(
        move |_lua, (table, key): (LuaValue, LuaValue)| -> Result<MultiValue, mlua::Error> {
            if let LuaValue::Table(table) = &table
                && let Some(metatable) = table.metatable()
                && metatable
                    .get::<bool>("__logiksmith_readonly")
                    .unwrap_or(false)
            {
                let next_fn: Function = metatable.get("__logiksmith_next")?;
                return next_fn.call((key,));
            }
            builtin_next.call((table, key))
        },
    )?;
    environment.set("next", safe_next)?;
    environment.set("seconds", duration_helper(lua, 1_000)?)?;
    environment.set("minutes", duration_helper(lua, 60_000)?)?;
    environment.set("hours", duration_helper(lua, 3_600_000)?)?;
    environment.set("days", duration_helper(lua, 86_400_000)?)?;
    let weekdays_backing = lua.create_table()?;
    for (index, name) in Weekday::ALL.iter().enumerate() {
        weekdays_backing.set(index + 1, name.to_string())?;
        weekdays_backing.set(name.to_string(), name.to_string())?;
    }
    environment.set("weekdays", readonly_proxy(lua, weekdays_backing)?)?;
    Ok(environment)
}

pub(crate) const READ_ONLY_ARGUMENT_MARKER: &str = "logiksmith read-only argument";

fn duration_helper(lua: &Lua, factor: u64) -> Result<Function, mlua::Error> {
    lua.create_function(move |_lua, value: LuaValue| {
        let number = match value {
            LuaValue::Integer(value) => value as f64,
            LuaValue::Number(value) => value,
            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "duration helper expects a positive finite number, got {}",
                    other.type_name()
                )));
            }
        };
        if !number.is_finite() || number <= 0.0 {
            return Err(mlua::Error::RuntimeError(
                "duration helper expects a positive finite number".to_owned(),
            ));
        }
        let milliseconds = number * factor as f64;
        if !milliseconds.is_finite()
            || milliseconds < 1.0
            || milliseconds.fract() != 0.0
            || milliseconds > u32::MAX as f64
        {
            return Err(mlua::Error::RuntimeError(
                "duration helper result must be a whole millisecond in range 1..=u32::MAX"
                    .to_owned(),
            ));
        }
        Ok(milliseconds as i64)
    })
}

fn readonly_proxy(lua: &Lua, backing: Table) -> Result<Table, mlua::Error> {
    let proxy = lua.create_table()?;
    let metatable = lua.create_table()?;
    metatable.set("__logiksmith_readonly", true)?;
    metatable.set("__index", backing.clone())?;
    metatable.set(
        "__newindex",
        lua.create_function(
            |_, (_table, _key, _value): (LuaValue, LuaValue, LuaValue)| {
                Err::<(), _>(mlua::Error::RuntimeError(
                    READ_ONLY_ARGUMENT_MARKER.to_owned(),
                ))
            },
        )?,
    )?;

    let pairs_backing = backing.clone();
    metatable.set(
        "__pairs",
        lua.create_function(move |lua, ()| {
            let entries = pairs_backing
                .pairs::<LuaValue, LuaValue>()
                .collect::<Result<Vec<_>, _>>()?;
            let index = Rc::new(Cell::new(0usize));
            let iterator =
                lua.create_function(move |_lua, (_state, _key): (LuaValue, LuaValue)| {
                    let current = index.get();
                    if current >= entries.len() {
                        return Ok(MultiValue::from_vec(vec![LuaValue::Nil]));
                    }
                    index.set(current + 1);
                    Ok(MultiValue::from_vec(vec![
                        entries[current].0.clone(),
                        entries[current].1.clone(),
                    ]))
                })?;
            Ok((iterator, LuaValue::Nil, LuaValue::Nil))
        })?,
    )?;

    let next_backing = backing;
    metatable.set(
        "__logiksmith_next",
        lua.create_function(move |_lua, key: LuaValue| {
            let entries = next_backing
                .pairs::<LuaValue, LuaValue>()
                .collect::<Result<Vec<_>, _>>()?;
            let index = if key.is_nil() {
                Some(0)
            } else {
                entries
                    .iter()
                    .position(|(entry_key, _)| entry_key.equals(&key).unwrap_or(false))
                    .map(|position| position + 1)
            };
            let Some(index) = index else {
                return Err(mlua::Error::RuntimeError(
                    "invalid key to 'next'".to_owned(),
                ));
            };
            if index >= entries.len() {
                Ok(MultiValue::from_vec(vec![LuaValue::Nil]))
            } else {
                Ok(MultiValue::from_vec(vec![
                    entries[index].0.clone(),
                    entries[index].1.clone(),
                ]))
            }
        })?,
    )?;
    proxy.set_metatable(Some(metatable));
    Ok(proxy)
}

fn install_instruction_hook(
    lua: &Lua,
    limits: &RuntimeLimits,
    budget_probe: Option<Arc<dyn BudgetProbe>>,
) {
    let count = Rc::new(Cell::new(0_u32));
    let maximum = limits.max_logic_instructions;
    let time_limit = limits.logic_handler_time_budget_ms;
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(1),
        move |_lua, _debug| {
            let next = count.get().saturating_add(1);
            count.set(next);
            if next >= maximum {
                Err(mlua::Error::RuntimeError(
                    INSTRUCTION_LIMIT_MARKER.to_owned(),
                ))
            } else if time_limit.is_some_and(|limit| {
                budget_probe.as_ref().is_some_and(|probe| probe.elapsed_ms() >= limit)
            }) {
                Err(mlua::Error::RuntimeError(
                    HANDLER_TIME_LIMIT_MARKER.to_owned(),
                ))
            } else {
                Ok(VmState::Continue)
            }
        },
    );
}

pub(crate) fn validate_logic_source_with_limits(
    source: &str,
    limits: &RuntimeLimits,
    budget_probe: Option<Arc<dyn BudgetProbe>>,
) -> Result<(), LogicError> {
    check_source_size_with_limits(source, limits)?;
    let lua = new_lua(limits).map_err(|error| map_lua_error(error, LuaPhase::Load))?;
    let environment =
        restricted_environment(&lua).map_err(|error| map_lua_error(error, LuaPhase::Load))?;
    install_instruction_hook(&lua, limits, budget_probe);

    let chunk = lua
        .load(source)
        .set_name("logic.source")
        .set_environment(environment.clone())
        .into_function()
        .map_err(|error| map_lua_error(error, LuaPhase::Syntax))?;
    chunk
        .call::<()>(())
        .map_err(|error| map_lua_error(error, LuaPhase::Load))?;
    match environment
        .get::<LuaValue>("handle")
        .map_err(|error| map_lua_error(error, LuaPhase::Load))?
    {
        LuaValue::Function(_) => Ok(()),
        value => Err(LogicError::Load {
            message: format!(
                "logic.source must define callable handle, got {}",
                value.type_name()
            ),
            line: None,
        }),
    }
}
