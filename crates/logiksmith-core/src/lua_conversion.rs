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

fn convert_result(
    endpoints: &[Endpoint],
    result: LuaValue,
    current_state: &TransientState,
    pending_timers: &BTreeMap<TimerName, PendingTimer>,
    now: MonotonicMs,
) -> Result<Transition, LogicError> {
    let result_table = match result {
        LuaValue::Nil => {
            return Ok(Transition {
                state: BTreeMap::new(),
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
        if !matches!(key.as_str(), "state" | "outputs" | "timers") {
            return Err(LogicError::InvalidResult {
                message: format!(
                    "unsupported result field {key:?}; only state, outputs, and timers are allowed"
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
    let mut state_patch = BTreeMap::new();
    if let Some(state_table) = state {
        for pair in state_table.pairs::<LuaValue, LuaValue>() {
            let (key, value) =
                pair.map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?;
            let key = match key {
                LuaValue::String(key) => key
                    .to_str()
                    .map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?
                    .to_owned(),
                value => {
                    return Err(LogicError::InvalidResult {
                        message: format!("state key must be a string, got {}", value.type_name()),
                        line: None,
                    });
                }
            };
            let value = convert_state_value(&key, value)?;
            if state_patch.insert(key.clone(), value).is_some() {
                return Err(LogicError::InvalidResult {
                    message: format!("state key {key:?} was returned more than once"),
                    line: None,
                });
            }
        }
    }
    merge_state(current_state, &state_patch).map_err(|error| LogicError::InvalidResult {
        message: error.to_string(),
        line: None,
    })?;

    let Some(outputs_table) = outputs else {
        let timers = convert_timers(timers, pending_timers, now)?;
        return Ok(Transition {
            state: state_patch,
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
    let timers = convert_timers(timers, pending_timers, now)?;
    Ok(Transition {
        state: state_patch,
        outputs,
        timers,
    })
}

fn convert_timers(
    timers: Option<Table>,
    pending: &BTreeMap<TimerName, PendingTimer>,
    now: MonotonicMs,
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
    if candidate.len() > MAX_PENDING_TIMERS {
        return Err(LogicError::InvalidResult {
            message: format!("pending timers exceed maximum of {MAX_PENDING_TIMERS}"),
            line: None,
        });
    }
    Ok(raw)
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
