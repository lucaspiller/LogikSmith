use std::{
    cell::RefCell,
    collections::BTreeMap,
    ffi::CStr,
    os::raw::{c_int, c_void},
    rc::Rc,
};

use mlua::ffi;

/// The bounded source trace captured for one completed Lua handler call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExecutionTrace {
    pub entries: Vec<TraceEntry>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TraceEntry {
    pub line: u32,
    pub locals: BTreeMap<String, TraceValue>,
}

/// A browser-safe representation of a Lua value. Tables intentionally use
/// string keys because the management API is an object-shaped diagnostic
/// projection; non-string Lua keys are rendered with a stable bracket form.
#[derive(Clone, Debug, PartialEq)]
pub enum TraceValue {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(String),
    Table {
        entries: BTreeMap<String, TraceValue>,
        truncated: bool,
    },
    Unsupported {
        kind: String,
        reason: String,
    },
}

#[derive(Clone)]
pub(crate) struct TraceCapture {
    inner: Rc<RefCell<TraceBuilder>>,
}

struct TraceBuilder {
    limits: TraceLimits,
    trace: ExecutionTrace,
    bytes: usize,
}

struct TraceBudget {
    remaining: usize,
    used: usize,
    exhausted: bool,
}

impl TraceBudget {
    fn new(remaining: usize) -> Self {
        Self {
            remaining,
            used: 0,
            exhausted: false,
        }
    }

    fn reserve(&mut self, bytes: usize) -> bool {
        if self.remaining < bytes {
            self.exhausted = true;
            return false;
        }
        self.remaining -= bytes;
        self.used = self.used.saturating_add(bytes);
        true
    }
}

#[derive(Clone, Copy)]
struct TraceLimits {
    entries: usize,
    locals: usize,
    depth: usize,
    table_entries: usize,
    string_bytes: usize,
    bytes: usize,
}

impl TraceCapture {
    pub(crate) fn new(limits: &crate::RuntimeLimits) -> Self {
        Self {
            inner: Rc::new(RefCell::new(TraceBuilder {
                limits: TraceLimits {
                    entries: limits.max_trace_entries_per_execution,
                    locals: limits.max_trace_locals_per_entry,
                    depth: limits.max_trace_value_depth,
                    table_entries: limits.max_trace_table_entries,
                    string_bytes: limits.max_trace_string_bytes,
                    bytes: limits.max_trace_bytes,
                },
                trace: ExecutionTrace::default(),
                bytes: 0,
            })),
        }
    }

    pub(crate) fn clear(&self) {
        let mut builder = self.inner.borrow_mut();
        builder.trace = ExecutionTrace::default();
        builder.bytes = 0;
    }

    pub(crate) fn finish(&self) -> ExecutionTrace {
        self.inner.borrow().trace.clone()
    }

    pub(crate) fn capture_line(&self, lua: &mlua::Lua, line: u32) {
        if self.inner.borrow().trace.truncated {
            return;
        }
        let capture = self.clone();
        let _: mlua::Result<()> = unsafe {
            lua.exec_raw((), move |state| {
                capture.capture_raw(state, line);
            })
        };
    }

    fn capture_raw(&self, state: *mut ffi::lua_State, line: u32) {
        let (limits, previous_bytes) = {
            let builder = self.inner.borrow();
            (builder.limits, builder.bytes)
        };
        if previous_bytes >= limits.bytes {
            self.inner.borrow_mut().trace.truncated = true;
            return;
        }
        if self.inner.borrow().trace.entries.len() >= limits.entries {
            self.inner.borrow_mut().trace.truncated = true;
            return;
        }
        let mut frame = unsafe { std::mem::zeroed::<ffi::lua_Debug>() };
        if unsafe { ffi::lua_getstack(state, 0, &mut frame) } == 0 {
            return;
        }

        let base = unsafe { ffi::lua_gettop(state) };
        let mut locals = BTreeMap::new();
        let mut budget = TraceBudget::new(limits.bytes.saturating_sub(previous_bytes));
        let mut local_index = 1_i32;
        let mut local_count = 0_usize;
        while local_count < limits.locals {
            let name_ptr = unsafe { ffi::lua_getlocal(state, &frame, local_index) };
            if name_ptr.is_null() {
                break;
            }
            let raw_name = unsafe { CStr::from_ptr(name_ptr) }.to_bytes();
            if raw_name.len() > limits.string_bytes {
                budget.exhausted = true;
                unsafe { ffi::lua_settop(state, ffi::lua_gettop(state) - 1) };
                break;
            }
            if !budget.reserve(raw_name.len().saturating_add(16)) {
                unsafe { ffi::lua_settop(state, ffi::lua_gettop(state) - 1) };
                break;
            }
            let name = String::from_utf8_lossy(raw_name).into_owned();
            let value_bytes_before = budget.used;
            let value = unsafe {
                value_from_stack(
                    state,
                    -1,
                    limits,
                    0,
                    &mut Vec::new(),
                    &mut budget,
                )
            };
            unsafe { ffi::lua_settop(state, ffi::lua_gettop(state) - 1) };
            if budget.exhausted && budget.used == value_bytes_before {
                break;
            }
            if !name.starts_with("(*") {
                locals.insert(name, value);
                local_count += 1;
            }
            local_index += 1;
            if budget.exhausted {
                break;
            }
        }
        let local_limit_hit = if budget.exhausted {
            false
        } else {
            let local_limit_hit = unsafe { ffi::lua_getlocal(state, &frame, local_index) };
            let local_limit_hit = !local_limit_hit.is_null();
            if local_limit_hit {
                unsafe { ffi::lua_settop(state, ffi::lua_gettop(state) - 1) };
            }
            local_limit_hit
        };
        unsafe { ffi::lua_settop(state, base) };

        if locals.is_empty() {
            if budget.exhausted {
                self.inner.borrow_mut().trace.truncated = true;
            }
            return;
        }
        let entry = TraceEntry { line, locals };
        let mut builder = self.inner.borrow_mut();
        if builder.trace.entries.len() >= builder.limits.entries {
            builder.trace.truncated = true;
            return;
        }
        builder.bytes = builder.bytes.saturating_add(budget.used);
        builder.trace.entries.push(entry);
        if local_limit_hit || budget.exhausted {
            builder.trace.truncated = true;
        }
    }
}

unsafe fn value_from_stack(
    state: *mut ffi::lua_State,
    index: c_int,
    limits: TraceLimits,
    depth: usize,
    ancestors: &mut Vec<*const c_void>,
    budget: &mut TraceBudget,
) -> TraceValue {
    let kind = unsafe { ffi::lua_type(state, index) };
    match kind {
        ffi::LUA_TNIL => {
            budget.reserve(4);
            TraceValue::Nil
        }
        ffi::LUA_TBOOLEAN => {
            budget.reserve(5);
            TraceValue::Boolean(unsafe { ffi::lua_toboolean(state, index) != 0 })
        }
        ffi::LUA_TNUMBER if unsafe { ffi::lua_isinteger(state, index) != 0 } => {
            budget.reserve(16);
            TraceValue::Integer(unsafe { ffi::lua_tointegerx(state, index, std::ptr::null_mut()) })
        }
        ffi::LUA_TNUMBER => {
            budget.reserve(16);
            let value = unsafe { ffi::lua_tonumberx(state, index, std::ptr::null_mut()) };
            if value.is_finite() {
                TraceValue::Number(value)
            } else {
                unsupported_value("number", "non-finite number cannot be serialized", budget)
            }
        }
        ffi::LUA_TSTRING => {
            let mut length = 0;
            let pointer = unsafe { ffi::lua_tolstring(state, index, &mut length) };
            if pointer.is_null() {
                unsupported_value("string", "string value was unavailable", budget)
            } else if length > limits.string_bytes {
                unsupported_value("string", "string exceeds trace byte limit", budget)
            } else {
                let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) };
                match std::str::from_utf8(bytes) {
                    Ok(value) if budget.reserve(value.len().saturating_add(8)) => {
                        TraceValue::String(value.to_owned())
                    }
                    Ok(_) => unsupported_value("string", "trace byte limit reached", budget),
                    Err(_) => unsupported_value("string", "string is not valid UTF-8", budget),
                }
            }
        }
        ffi::LUA_TTABLE => {
            let pointer = unsafe { ffi::lua_topointer(state, index) };
            if depth >= limits.depth {
                return unsupported_value("table", "table nesting limit reached", budget);
            }
            if ancestors.contains(&pointer) {
                return unsupported_value("table", "cyclic table value", budget);
            }
            if !budget.reserve(24) {
                return unsupported_value("table", "trace byte limit reached", budget);
            }
            ancestors.push(pointer);
            let value = unsafe { table_value(state, index, limits, depth, ancestors, budget) };
            ancestors.pop();
            value
        }
        other => unsupported_value(
            &unsafe { type_name(state, other) },
            "value type is not safely serializable",
            budget,
        ),
    }
}

unsafe fn table_value(
    state: *mut ffi::lua_State,
    index: c_int,
    limits: TraceLimits,
    depth: usize,
    ancestors: &mut Vec<*const c_void>,
    budget: &mut TraceBudget,
) -> TraceValue {
    let base = unsafe { ffi::lua_gettop(state) };
    let table_index = unsafe { ffi::lua_absindex(state, index) };
    let mut source_index = table_index;
    // The read-only API proxies expose their actual values through __index.
    // Follow that backing table so event/input/meta/state/context are useful in
    // the trace while retaining the proxy's public read-only semantics.
    if unsafe { ffi::lua_getmetatable(state, table_index) } != 0 {
        let metatable = unsafe { ffi::lua_gettop(state) };
        let key = b"__index\0";
        unsafe { ffi::lua_getfield(state, metatable, key.as_ptr().cast()) };
        if unsafe { ffi::lua_type(state, -1) } == ffi::LUA_TTABLE {
            source_index = unsafe { ffi::lua_absindex(state, -1) };
        } else {
            unsafe { ffi::lua_settop(state, base) };
        }
    }

    let mut entries = BTreeMap::new();
    let mut truncated = false;
    unsafe { ffi::lua_pushnil(state) };
    while unsafe { ffi::lua_next(state, source_index) } != 0 {
        if entries.len() >= limits.table_entries || budget.exhausted {
            truncated = true;
            unsafe { ffi::lua_settop(state, base) };
            break;
        }
        let Some(key) = (unsafe { key_from_stack(state, -2, limits, budget) }) else {
            truncated = true;
            unsafe { ffi::lua_settop(state, base) };
            break;
        };
        let value = unsafe {
            value_from_stack(state, -1, limits, depth + 1, ancestors, budget)
        };
        if budget.exhausted {
            truncated = true;
            unsafe { ffi::lua_settop(state, base) };
            break;
        }
        entries.insert(key, value);
        unsafe { ffi::lua_settop(state, ffi::lua_gettop(state) - 1) };
    }
    unsafe { ffi::lua_settop(state, base) };
    TraceValue::Table { entries, truncated }
}

unsafe fn key_from_stack(
    state: *mut ffi::lua_State,
    index: c_int,
    limits: TraceLimits,
    budget: &mut TraceBudget,
) -> Option<String> {
    match unsafe { ffi::lua_type(state, index) } {
        ffi::LUA_TSTRING => {
            let mut length = 0;
            let pointer = unsafe { ffi::lua_tolstring(state, index, &mut length) };
            if pointer.is_null() {
                let key = "[string]".to_owned();
                budget.reserve(key.len().saturating_add(16)).then_some(key)
            } else if length > limits.string_bytes {
                budget.exhausted = true;
                None
            } else {
                let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) };
                if !budget.reserve(length.saturating_add(16)) {
                    return None;
                }
                let key = std::str::from_utf8(bytes).ok()?.to_owned();
                Some(key)
            }
        }
        ffi::LUA_TNUMBER if unsafe { ffi::lua_isinteger(state, index) != 0 } => {
            let key = format!("[{}]", unsafe {
                ffi::lua_tointegerx(state, index, std::ptr::null_mut())
            });
            budget.reserve(key.len().saturating_add(16)).then_some(key)
        }
        ffi::LUA_TNUMBER => {
            let key = format!("[{}]", unsafe {
                ffi::lua_tonumberx(state, index, std::ptr::null_mut())
            });
            budget.reserve(key.len().saturating_add(16)).then_some(key)
        }
        ffi::LUA_TBOOLEAN => {
            let key = format!("[{}]", unsafe { ffi::lua_toboolean(state, index) != 0 });
            budget.reserve(key.len().saturating_add(16)).then_some(key)
        }
        other => {
            let key = format!("[{}]", unsafe { type_name(state, other) });
            budget.reserve(key.len().saturating_add(16)).then_some(key)
        }
    }
}

unsafe fn type_name(state: *mut ffi::lua_State, kind: c_int) -> String {
    let pointer = unsafe { ffi::lua_typename(state, kind) };
    if pointer.is_null() {
        "unknown".to_owned()
    } else {
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    }
}

fn unsupported_value(kind: &str, reason: &str, budget: &mut TraceBudget) -> TraceValue {
    budget.reserve(kind.len().saturating_add(reason.len()).saturating_add(16));
    TraceValue::Unsupported {
        kind: kind.to_owned(),
        reason: reason.to_owned(),
    }
}
