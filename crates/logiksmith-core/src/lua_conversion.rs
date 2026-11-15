fn typed_value_to_lua(value: TypedValue) -> LuaValue {
    match value.value() {
        Value::Bool(value) => LuaValue::Boolean(value),
        Value::Percent(value) => LuaValue::Integer(i64::from(value)),
        Value::Temperature(value) => LuaValue::Number(f64::from(value) / 100.0),
    }
}

fn state_value_to_lua(lua: &Lua, value: &StateValue) -> Result<LuaValue, mlua::Error> {
    match value {
        StateValue::Bool(value) => Ok(LuaValue::Boolean(*value)),
        StateValue::Integer(value) => Ok(LuaValue::Integer(*value)),
        StateValue::Number(value) => Ok(LuaValue::Number(*value)),
        StateValue::String(value) => Ok(LuaValue::String(lua.create_string(value)?)),
    }
}

fn convert_state_value(key: &str, value: LuaValue) -> Result<StateValue, LogicError> {
    let state_value = match value {
        LuaValue::Boolean(value) => StateValue::Bool(value),
        LuaValue::Integer(value) => StateValue::Integer(value),
        LuaValue::Number(value) if value.is_finite() => StateValue::Number(value),
        LuaValue::Number(_) => {
            return Err(LogicError::InvalidResult {
                message: format!("state value {key:?} must be finite"),
                line: None,
            });
        }
        LuaValue::String(value) => StateValue::String(
            value
                .to_str()
                .map_err(|error| LogicError::InvalidResult {
                    message: format!("state string {key:?} is not valid UTF-8: {error}"),
                    line: None,
                })?
                .to_owned(),
        ),
        value => {
            return Err(LogicError::InvalidResult {
                message: format!(
                    "state value {key:?} must be boolean, integer, finite number, or string, got {}",
                    value.type_name()
                ),
                line: None,
            });
        }
    };
    validate_state_entry(key, &state_value).map_err(|error| LogicError::InvalidResult {
        message: error.to_string(),
        line: None,
    })?;
    Ok(state_value)
}

fn convert_state_map(table: Option<Table>, field: &str) -> Result<StatePatch, LogicError> {
    let mut values = BTreeMap::new();
    let Some(table) = table else {
        return Ok(values);
    };
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair.map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?;
        let key = match key {
            LuaValue::String(key) => key
                .to_str()
                .map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?
                .to_owned(),
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!("{field} key must be a string, got {}", value.type_name()),
                    line: None,
                });
            }
        };
        let value = convert_state_value(&key, value)?;
        if values.insert(key.clone(), value).is_some() {
            return Err(LogicError::InvalidResult {
                message: format!("{field} key {key:?} was returned more than once"),
                line: None,
            });
        }
    }
    Ok(values)
}

fn convert_persistent_delete(table: Option<Table>) -> Result<std::collections::BTreeSet<String>, LogicError> {
    let mut entries = BTreeMap::new();
    let Some(table) = table else {
        return Ok(std::collections::BTreeSet::new());
    };
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (index, value) = pair.map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?;
        let index = match index {
            LuaValue::Integer(index) if index > 0 => index,
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!("persistent.delete must be an array, got index {}", value.type_name()),
                    line: None,
                });
            }
        };
        let key = match value {
            LuaValue::String(key) => key
                .to_str()
                .map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?
                .to_owned(),
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!("persistent.delete entries must be strings, got {}", value.type_name()),
                    line: None,
                });
            }
        };
        if entries.insert(index, key).is_some() {
            return Err(LogicError::InvalidResult {
                message: "persistent.delete contains a duplicate array index".to_owned(),
                line: None,
            });
        }
    }
    let mut keys = std::collections::BTreeSet::new();
    for (offset, (index, key)) in entries.into_iter().enumerate() {
        if index != i64::try_from(offset + 1).unwrap_or(i64::MAX) {
            return Err(LogicError::InvalidResult {
                message: "persistent.delete must be a dense array".to_owned(),
                line: None,
            });
        }
        validate_state_key(&key).map_err(|error| LogicError::InvalidResult {
            message: error.to_string(),
            line: None,
        })?;
        if !keys.insert(key.clone()) {
            return Err(LogicError::InvalidResult {
                message: format!("persistent.delete key {key:?} was returned more than once"),
                line: None,
            });
        }
    }
    Ok(keys)
}

fn convert_persistent_patch(table: Option<Table>) -> Result<PersistentPatch, LogicError> {
    let mut set = None;
    let mut delete = None;
    if let Some(table) = table {
        for pair in table.pairs::<LuaValue, LuaValue>() {
            let (key, value) = pair.map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?;
            let key = match key {
                LuaValue::String(key) => key
                    .to_str()
                    .map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?
                    .to_owned(),
                value => {
                    return Err(LogicError::InvalidResult {
                        message: format!("persistent patch field must be a string, got {}", value.type_name()),
                        line: None,
                    });
                }
            };
            let target = match key.as_str() {
                "set" => &mut set,
                "delete" => &mut delete,
                _ => {
                    return Err(LogicError::InvalidResult {
                        message: format!("unsupported persistent patch field {key:?}; only set and delete are allowed"),
                        line: None,
                    });
                }
            };
            match value {
                LuaValue::Table(table) if target.is_none() => *target = Some(table),
                LuaValue::Table(_) => {
                    return Err(LogicError::InvalidResult {
                        message: format!("persistent patch contains duplicate {key} fields"),
                        line: None,
                    });
                }
                value => {
                    return Err(LogicError::InvalidResult {
                        message: format!("persistent.{key} must be a table, got {}", value.type_name()),
                        line: None,
                    });
                }
            }
        }
    }
    let set = convert_state_map(set, "persistent.set")?;
    let delete = convert_persistent_delete(delete)?;
    if let Some(key) = set.keys().find(|key| delete.contains(*key)) {
        return Err(LogicError::InvalidResult {
            message: format!("persistent key {key:?} cannot be both set and deleted"),
            line: None,
        });
    }
    Ok(PersistentPatch { set, delete })
}

fn convert_result(
    endpoints: &[Endpoint],
    result: LuaValue,
    current_state: &TransientState,
    current_persistent_state: &PersistentState,
    persistent_state_enabled: bool,
    pending_timers: &BTreeMap<TimerName, PendingTimer>,
    now: MonotonicMs,
    limits: &RuntimeLimits,
) -> Result<Transition, LogicError> {
    let result_table = match result {
        LuaValue::Nil => {
            return Ok(Transition {
                state: BTreeMap::new(),
                persistent: PersistentPatch::default(),
                outputs: Vec::new(),
                timers: Vec::new(),
            });
        }
        LuaValue::Table(table) => table,
        value => {
            return Err(LogicError::InvalidResult {
                message: format!(
                    "handle result must be nil or a table, got {}",
                    value.type_name()
                ),
                line: None,
            });
        }
    };

    let mut state: Option<Table> = None;
    let mut outputs: Option<Table> = None;
    let mut timers: Option<Table> = None;
    let mut persistent: Option<Table> = None;
    for pair in result_table.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair.map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?;
        let key = match key {
            LuaValue::String(key) => key
                .to_str()
                .map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?
                .to_owned(),
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!("unsupported result field key of type {}", value.type_name()),
                    line: None,
                });
            }
        };
        if !matches!(key.as_str(), "state" | "outputs" | "timers" | "persistent") {
            return Err(LogicError::InvalidResult {
                message: format!(
                    "unsupported result field {key:?}; only state, outputs, timers, and persistent are allowed"
                ),
                line: None,
            });
        }
        match value {
            LuaValue::Table(table) if key == "state" && state.is_none() => state = Some(table),
            LuaValue::Table(table) if key == "outputs" && outputs.is_none() => {
                outputs = Some(table)
            }
            LuaValue::Table(table) if key == "timers" && timers.is_none() => timers = Some(table),
            LuaValue::Table(table) if key == "persistent" && persistent.is_none() => {
                persistent = Some(table)
            }
            LuaValue::Table(_) => {
                return Err(LogicError::InvalidResult {
                    message: format!("result contains duplicate {key} fields"),
                    line: None,
                });
            }
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!("{key} must be a table, got {}", value.type_name()),
                    line: None,
                });
            }
        }
    }
    let state_patch = convert_state_map(state, "state")?;
    let persistent_patch = convert_persistent_patch(persistent)?;
    let persistent_entries = persistent_patch.set.len() + persistent_patch.delete.len();
    if !persistent_state_enabled && persistent_entries > 0 {
        return Err(LogicError::InvalidResult {
            message: "persistent state effects are disabled for this runtime".to_owned(),
            line: None,
        });
    }
    let state_entries = state_patch.len().saturating_add(persistent_entries);
    if state_entries > limits.max_state_patch_entries_per_execution {
        return Err(LogicError::InvalidResult {
            message: format!(
                "state and persistent patch has {}; maximum is {}",
                state_entries,
                limits.max_state_patch_entries_per_execution
            ),
            line: None,
        });
    }
    merge_combined_state_with_limits(
        current_state,
        &state_patch,
        current_persistent_state,
        &persistent_patch,
        limits,
    )
    .map_err(|error| LogicError::InvalidResult {
        message: error.to_string(),
        line: None,
    })?;

    let Some(outputs_table) = outputs else {
        let timers = convert_timers(timers, pending_timers, now, limits)?;
        check_combined_effects(state_entries, 0, timers.len(), limits)?;
        return Ok(Transition {
            state: state_patch,
            persistent: persistent_patch,
            outputs: Vec::new(),
            timers,
        });
    };

    // Keep temporary slots until every returned field has passed validation;
    // this is the all-or-nothing boundary before any host write is possible.
    let mut values: Vec<Option<TypedValue>> = vec![None; endpoints.len()];
    for pair in outputs_table.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair.map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?;
        let key = match key {
            LuaValue::String(key) => key
                .to_str()
                .map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?
                .to_owned(),
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!("output name must be a string, got {}", value.type_name()),
                    line: None,
                });
            }
        };
        let (index, endpoint) = endpoints
            .iter()
            .enumerate()
            .find(|(_, endpoint)| {
                endpoint.direction == EndpointDirection::Output && endpoint.name.as_str() == key
            })
            .ok_or_else(|| LogicError::InvalidResult {
                message: format!("unknown output endpoint {key}"),
                line: None,
            })?;
        let typed = lua_to_typed_value(endpoint, value)?;
        if values[index].replace(typed).is_some() {
            return Err(LogicError::InvalidResult {
                message: format!("output endpoint {key} was returned more than once"),
                line: None,
            });
        }
    }

    let outputs = endpoints
        .iter()
        .enumerate()
        .filter_map(|(index, endpoint)| {
            (endpoint.direction == EndpointDirection::Output)
                .then(|| {
                    values[index].map(|value| OutputEffect {
                        endpoint: endpoint.name.clone(),
                        value,
                    })
                })
                .flatten()
        })
        .collect::<Vec<_>>();
    let timers = convert_timers(timers, pending_timers, now, limits)?;
    check_combined_effects(state_entries, outputs.len(), timers.len(), limits)?;
    Ok(Transition {
        state: state_patch,
        persistent: persistent_patch,
        outputs,
        timers,
    })
}

fn convert_timers(
    timers: Option<Table>,
    pending: &BTreeMap<TimerName, PendingTimer>,
    now: MonotonicMs,
    limits: &RuntimeLimits,
) -> Result<Vec<TimerEffect>, LogicError> {
    let Some(timers) = timers else {
        return Ok(Vec::new());
    };
    let mut raw = Vec::new();
    for pair in timers.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair.map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?;
        let key = match key {
            LuaValue::String(key) => key
                .to_str()
                .map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?
                .to_owned(),
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!("timer name must be a string, got {}", value.type_name()),
                    line: None,
                });
            }
        };
        let name = TimerName::new(key.clone()).map_err(|error| LogicError::InvalidResult {
            message: format!("invalid timer name {key:?}: {error}"),
            line: None,
        })?;
        let action = match value {
            LuaValue::Boolean(false) => match pending.get(&name) {
                Some(timer) => TimerAction::Cancelled {
                    previous_due_at: timer.due_at,
                },
                None => TimerAction::CancelNoop,
            },
            LuaValue::Table(schedule) => {
                let mut after: Option<u32> = None;
                for pair in schedule.pairs::<LuaValue, LuaValue>() {
                    let (field, value) =
                        pair.map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?;
                    let field = match field {
                        LuaValue::String(field) => field
                            .to_str()
                            .map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?
                            .to_owned(),
                        value => {
                            return Err(LogicError::InvalidResult {
                                message: format!(
                                    "timer {name} field must be a string, got {}",
                                    value.type_name()
                                ),
                                line: None,
                            });
                        }
                    };
                    if field != "after" {
                        return Err(LogicError::InvalidResult {
                            message: format!(
                                "timer {name} schedule only accepts after, got {field:?}"
                            ),
                            line: None,
                        });
                    }
                    let after_value = lua_duration_ms(value)?;
                    if after.replace(after_value).is_some() {
                        return Err(LogicError::InvalidResult {
                            message: format!("timer {name} contains duplicate after fields"),
                            line: None,
                        });
                    }
                }
                let after_ms = after.ok_or_else(|| LogicError::InvalidResult {
                    message: format!("timer {name} schedule requires after"),
                    line: None,
                })?;
                let due_at =
                    now.checked_add(after_ms)
                        .ok_or_else(|| LogicError::InvalidResult {
                            message: format!("timer {name} deadline overflows MonotonicMs"),
                            line: None,
                        })?;
                match pending.get(&name) {
                    Some(timer) => TimerAction::Replaced {
                        previous_due_at: timer.due_at,
                        after_ms,
                        due_at,
                    },
                    None => TimerAction::Scheduled { after_ms, due_at },
                }
            }
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!(
                        "timer {name} must be false or a schedule table, got {}",
                        value.type_name()
                    ),
                    line: None,
                });
            }
        };
        raw.push(TimerEffect { name, action });
    }
    raw.sort_by(|left, right| left.name.cmp(&right.name));
    let mut candidate = pending.clone();
    for effect in &raw {
        match effect.action {
            TimerAction::Scheduled { after_ms, due_at }
            | TimerAction::Replaced {
                after_ms, due_at, ..
            } => {
                candidate.insert(
                    effect.name.clone(),
                    PendingTimer {
                        name: effect.name.clone(),
                        scheduled_at: now,
                        due_at,
                        scheduled_logic_revision: 0,
                    },
                );
                let _ = after_ms;
            }
            TimerAction::Cancelled { .. } | TimerAction::CancelNoop => {
                candidate.remove(&effect.name);
            }
        }
    }
    if raw.len() > limits.max_timer_effects_per_execution {
        return Err(LogicError::InvalidResult {
            message: format!(
                "timer effects exceed maximum of {}",
                limits.max_timer_effects_per_execution
            ),
            line: None,
        });
    }
    if candidate.len() > limits.max_pending_timers_per_block {
        return Err(LogicError::InvalidResult {
            message: format!(
                "pending timers exceed maximum of {}",
                limits.max_pending_timers_per_block
            ),
            line: None,
        });
    }
    Ok(raw)
}

fn check_combined_effects(
    state: usize,
    outputs: usize,
    timers: usize,
    limits: &RuntimeLimits,
) -> Result<(), LogicError> {
    if outputs > limits.max_output_effects_per_execution {
        return Err(LogicError::InvalidResult {
            message: format!(
                "output effects exceed maximum of {}",
                limits.max_output_effects_per_execution
            ),
            line: None,
        });
    }
    let total = state.saturating_add(outputs).saturating_add(timers);
    if total > limits.max_combined_effects_per_execution {
        return Err(LogicError::InvalidResult {
            message: format!(
                "combined effects exceed maximum of {}",
                limits.max_combined_effects_per_execution
            ),
            line: None,
        });
    }
    Ok(())
}

fn lua_duration_ms(value: LuaValue) -> Result<u32, LogicError> {
    let number = match value {
        LuaValue::Integer(value) if (1..=i64::from(u32::MAX)).contains(&value) => {
            return Ok(value as u32);
        }
        LuaValue::Integer(value) => value as f64,
        LuaValue::Number(value) => value,
        value => {
            return Err(LogicError::InvalidResult {
                message: format!(
                    "timer after must be a positive finite whole millisecond, got {}",
                    value.type_name()
                ),
                line: None,
            });
        }
    };
    if !number.is_finite() || number <= 0.0 || number.fract() != 0.0 || number > u32::MAX as f64 {
        return Err(LogicError::InvalidResult {
            message:
                "timer after must be a positive finite whole millisecond in range 1..=u32::MAX"
                    .to_owned(),
            line: None,
        });
    }
    Ok(number as u32)
}

fn lua_to_typed_value(endpoint: &Endpoint, value: LuaValue) -> Result<TypedValue, LogicError> {
    match endpoint.dpt {
        dpt if dpt.is_bool() => match value {
            LuaValue::Boolean(value) => Ok(TypedValue::bool(value)),
            value => Err(LogicError::InvalidResult {
                message: format!(
                    "output {} expects a boolean, got {}",
                    endpoint.name,
                    value.type_name()
                ),
                line: None,
            }),
        },
        dpt if dpt.is_percent() => match value {
            LuaValue::Integer(value) if (0..=100).contains(&value) => {
                Ok(TypedValue::percent(value as u8).expect("bounded percentage"))
            }
            LuaValue::Integer(value) => Err(LogicError::InvalidResult {
                message: format!(
                    "output {} percentage {value} must be in range 0..=100",
                    endpoint.name
                ),
                line: None,
            }),
            LuaValue::Number(value) if value.is_nan() || value.is_infinite() => {
                Err(LogicError::InvalidResult {
                    message: format!(
                        "output {} percentage must be finite, got {value}",
                        endpoint.name
                    ),
                    line: None,
                })
            }
            LuaValue::Number(value) => Err(LogicError::InvalidResult {
                message: format!(
                    "output {} percentage must be an integer Lua number, got {value}",
                    endpoint.name
                ),
                line: None,
            }),
            value => Err(LogicError::InvalidResult {
                message: format!(
                    "output {} expects an integer percentage, got {}",
                    endpoint.name,
                    value.type_name()
                ),
                line: None,
            }),
        },
        dpt if dpt.is_temperature() => match value {
            LuaValue::Integer(value) => TypedValue::temperature(value as f64).map_err(|error| {
                LogicError::InvalidResult {
                    message: format!("output {} temperature: {error}", endpoint.name),
                    line: None,
                }
            }),
            LuaValue::Number(value) => TypedValue::temperature(value).map_err(|error| {
                LogicError::InvalidResult {
                    message: format!("output {} temperature: {error}", endpoint.name),
                    line: None,
                }
            }),
            value => Err(LogicError::InvalidResult {
                message: format!(
                    "output {} expects a finite temperature number, got {}",
                    endpoint.name,
                    value.type_name()
                ),
                line: None,
            }),
        },
        _ => Err(LogicError::InvalidResult {
            message: format!(
                "output {} uses unsupported DPT {}",
                endpoint.name, endpoint.dpt
            ),
            line: None,
        }),
    }
}
