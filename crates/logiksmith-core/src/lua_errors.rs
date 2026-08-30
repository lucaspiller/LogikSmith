fn map_lua_error(error: mlua::Error, phase: LuaPhase) -> LogicError {
    let text = truncate_message(error.to_string());
    let line = source_line(&text);
    if contains_instruction_marker(&error) {
        return LogicError::InstructionLimit {
            message: text,
            line,
        };
    }
    if error.to_string().contains(HANDLER_TIME_LIMIT_MARKER) {
        return LogicError::HandlerTimeLimit {
            message: text,
            line,
        };
    }
    if contains_memory_error(&error) {
        return LogicError::MemoryLimit {
            message: text,
            line,
        };
    }
    match phase {
        LuaPhase::Syntax => LogicError::Syntax {
            message: text,
            line,
        },
        LuaPhase::Load => LogicError::Load {
            message: text,
            line,
        },
        LuaPhase::Runtime => LogicError::Runtime {
            message: text,
            line,
        },
        LuaPhase::InvalidResult => LogicError::InvalidResult {
            message: text,
            line,
        },
    }
}

fn contains_instruction_marker(error: &mlua::Error) -> bool {
    if error.to_string().contains(INSTRUCTION_LIMIT_MARKER) {
        return true;
    }
    match error {
        mlua::Error::CallbackError { cause, .. } => contains_instruction_marker(cause),
        mlua::Error::BadArgument { cause, .. } => contains_instruction_marker(cause),
        _ => false,
    }
}

fn contains_memory_error(error: &mlua::Error) -> bool {
    if matches!(error, mlua::Error::MemoryError(_)) {
        return true;
    }
    match error {
        mlua::Error::CallbackError { cause, .. } => contains_memory_error(cause),
        mlua::Error::BadArgument { cause, .. } => contains_memory_error(cause),
        _ => false,
    }
}

fn truncate_message(message: String) -> String {
    const MAX_DIAGNOSTIC_BYTES: usize = 4096;
    if message.len() <= MAX_DIAGNOSTIC_BYTES {
        return message;
    }
    let mut end = MAX_DIAGNOSTIC_BYTES;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &message[..end])
}

fn source_line(message: &str) -> Option<usize> {
    let bytes = message.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index < bytes.len()
            && bytes[index] == b':'
            && let Ok(line) = message[start..index].parse::<usize>()
        {
            return Some(line);
        }
    }
    None
}
