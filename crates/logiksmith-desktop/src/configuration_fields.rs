#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct WebConfig {
    pub listen_ip: IpAddr,
    pub listen_port: u16,
}

impl WebConfig {
    pub fn new(listen_ip: IpAddr, listen_port: u16) -> Result<Self, ConfigError> {
        if listen_port == 0 {
            return Err(field("web.listen_port", "must be in range 1..=65535"));
        }
        Ok(Self {
            listen_ip,
            listen_port,
        })
    }

    pub fn socket_addr(self) -> SocketAddr {
        SocketAddr::new(self.listen_ip, self.listen_port)
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub gateway_ip: IpAddr,
    pub gateway_port: u16,
    pub local_ip: Option<IpAddr>,
}

#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub python: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub struct LoggingConfig {
    pub level: LevelFilter,
    pub bridge_level: LevelFilter,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawConfig {
    knx: RawKnxConfig,
    pub(crate) bridge: RawBridgeConfig,
    logging: RawLoggingConfig,
    web: RawWebConfig,
    time: RawTimeConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTimeConfig {
    timezone: String,
    latitude: Option<f64>,
    longitude: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKnxConfig {
    connection_type: String,
    gateway_ip: String,
    gateway_port: u32,
    local_ip: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawBridgeConfig {
    pub(crate) python: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLoggingConfig {
    level: String,
    bridge_level: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWebConfig {
    listen_ip: String,
    listen_port: u32,
}

fn field(field: impl Into<String>, message: impl Into<String>) -> ConfigError {
    ConfigError::Field {
        field: field.into(),
        message: message.into(),
    }
}

fn parse_ip(field_name: &str, value: &str) -> Result<IpAddr, ConfigError> {
    value
        .parse()
        .map_err(|error| field(field_name, format!("{error}")))
}

fn parse_level(field_name: &str, value: &str) -> Result<LevelFilter, ConfigError> {
    LevelFilter::from_str(value).map_err(|_| {
        field(
            field_name,
            "must be one of off, error, warn, info, debug, or trace",
        )
    })
}

fn parse_dpt(path: &str, value: &str) -> Result<Dpt, FieldError> {
    let dpt = Dpt::parse(value).map_err(|error| FieldError {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    if !dpt.is_supported() {
        return Err(FieldError {
            path: path.to_owned(),
            message: "must be 1.001, 5.001, or 9.001".to_owned(),
        });
    }
    if dpt.to_string() != value {
        return Err(FieldError {
            path: path.to_owned(),
            message: "must use canonical DPT form".to_owned(),
        });
    }
    Ok(dpt)
}

pub(crate) fn endpoint_name(path: &str, value: &str) -> Result<EndpointName, FieldError> {
    value.parse::<EndpointName>().map_err(|error| FieldError {
        path: path.to_owned(),
        message: error.to_string(),
    })
}

const MAX_LOGIC_SOURCE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_EXTERNAL_SOURCES: usize = 64;
pub(crate) const MAX_EXTERNAL_VALUES_AND_BINDINGS: usize = 256;
pub(crate) const MAX_HTTP_BODY_BYTES: usize = 64 * 1024;
const MAX_HTTP_URL_BYTES: usize = 2 * 1024;
const MAX_JSON_POINTER_BYTES: usize = 256;
const MAX_HTTP_HEADERS: usize = 16;
const MIN_POLL_INTERVAL_SECONDS: u64 = 1;
const MAX_POLL_INTERVAL_SECONDS: u64 = 24 * 60 * 60;
const MIN_REQUEST_TIMEOUT_MILLIS: u64 = 100;
const MAX_REQUEST_TIMEOUT_MILLIS: u64 = 30 * 1000;
const MAX_FRESHNESS_SECONDS: u64 = 7 * 24 * 60 * 60;

fn validate_json_pointer(path: &str, pointer: &str, errors: &mut Vec<FieldError>) -> bool {
    if pointer.len() > MAX_JSON_POINTER_BYTES {
        errors.push(FieldError {
            path: path.to_owned(),
            message: format!("must not exceed {MAX_JSON_POINTER_BYTES} bytes"),
        });
        return false;
    }
    if pointer.is_empty() {
        return true;
    }
    if !pointer.starts_with('/') {
        errors.push(FieldError {
            path: path.to_owned(),
            message: "must be empty or an RFC 6901 pointer beginning with '/'".to_owned(),
        });
        return false;
    }
    let mut valid = true;
    for token in pointer.split('/').skip(1) {
        let bytes = token.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'~' {
                if index + 1 >= bytes.len() || !matches!(bytes[index + 1], b'0' | b'1') {
                    valid = false;
                    break;
                }
                index += 2;
            } else {
                index += 1;
            }
        }
        if !valid {
            break;
        }
    }
    if !valid {
        errors.push(FieldError {
            path: path.to_owned(),
            message: "must use valid RFC 6901 '~0' and '~1' escapes".to_owned(),
        });
    }
    valid
}

fn parse_source_duration(path: &str, value: &str, errors: &mut Vec<FieldError>) -> Option<std::time::Duration> {
    let Some(seconds) = parse_duration_field(path, value, false, errors) else {
        return None;
    };
    u64::try_from(seconds).ok().map(std::time::Duration::from_secs)
}

fn parse_timeout_duration(
    path: &str,
    value: &str,
    errors: &mut Vec<FieldError>,
) -> Option<std::time::Duration> {
    let parsed = match value.strip_suffix("ms") {
        Some(millis) if !millis.is_empty() && millis.bytes().all(|byte| byte.is_ascii_digit()) =>
            millis.parse::<u64>().ok().map(std::time::Duration::from_millis),
        _ => parse_source_duration(path, value, errors),
    };
    let Some(duration) = parsed else { return None; };
    if !(std::time::Duration::from_millis(MIN_REQUEST_TIMEOUT_MILLIS)..=std::time::Duration::from_millis(MAX_REQUEST_TIMEOUT_MILLIS)).contains(&duration) {
        errors.push(FieldError {
            path: path.to_owned(),
            message: "must be between 100ms and 30s".to_owned(),
        });
        return None;
    }
    Some(duration)
}

fn source_name(path: &str, value: &str, errors: &mut Vec<FieldError>) -> Option<String> {
    match endpoint_name(path, value) {
        Ok(name) if name.as_str() == value => Some(value.to_owned()),
        Ok(_) => {
            errors.push(FieldError {
                path: path.to_owned(),
                message: "must use canonical identifier form".to_owned(),
            });
            None
        }
        Err(error) => {
            errors.push(error);
            None
        }
    }
}

fn validate_header(
    path: &str,
    header: &HttpHeader,
    errors: &mut Vec<FieldError>,
) -> Option<(String, String)> {
    if header.name.is_empty() || header.name.len() > 128 || header.name.bytes().any(|byte| byte <= 32 || byte >= 127 || byte == b':') {
        errors.push(FieldError {
            path: format!("{path}.name"),
            message: "must be a valid non-empty HTTP header name".to_owned(),
        });
        return None;
    }
    if header.value.is_some() == header.value_env.is_some() {
        errors.push(FieldError {
            path: path.to_owned(),
            message: "must set exactly one of value or value_env".to_owned(),
        });
        return None;
    }
    let value = match (&header.value, &header.value_env) {
        (Some(value), None) => value.clone(),
        (None, Some(variable)) if !variable.is_empty() => match std::env::var(variable) {
            Ok(value) => value,
            Err(_) => {
                errors.push(FieldError {
                    path: format!("{path}.value_env"),
                    message: format!("environment variable {variable:?} is not set"),
                });
                return None;
            }
        },
        _ => {
            errors.push(FieldError {
                path: format!("{path}.value_env"),
                message: "must not be empty".to_owned(),
            });
            return None;
        }
    };
    if value.bytes().any(|byte| byte == b'\r' || byte == b'\n') {
        errors.push(FieldError {
            path: path.to_owned(),
            message: "must not contain CR or LF".to_owned(),
        });
        return None;
    }
    Some((header.name.clone(), value))
}

// ---------------------------------------------------------------------------
// Schedule document validation
/// Computes the restart token for a document. Schedule definitions are
/// structure, so they contribute; block source and enabled status are live
/// fields that stay out of the token.
pub fn structural_revision(document: &AutomationDocument) -> u64 {
    let mut structure = document.clone();
    for block in &mut structure.blocks {
        block.source.clear();
        // The persisted revision changes when source changes, but source-only
        // edits are hot-activatable and must not require a restart.
        block.revision = 1;
        // Enabled status is a live, non-structural setting. Keep it out of
        // the restart token so an enable/disable batch can hot-apply.
        block.enabled = true;
    }
    let bytes = toml::to_string(&structure).unwrap_or_default();
    automation_revision(bytes.as_bytes())
}

/// Parses `HH:MM` or `HH:MM:SS` into a canonical second-resolution local time.
/// The two-digit canonical form is required so stored values round-trip.
fn parse_local_time(path: &str, value: &str, errors: &mut Vec<FieldError>) -> Option<LocalTime> {
    let parts: Vec<_> = value.split(':').collect();
    let valid_shape = matches!(parts.len(), 2 | 3)
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_digit()));
    if !valid_shape {
        errors.push(FieldError {
            path: path.to_owned(),
            message: "must be a canonical local time HH:MM or HH:MM:SS".to_owned(),
        });
        return None;
    }
    let hour = parts[0].parse::<u8>().ok();
    let minute = parts[1].parse::<u8>().ok();
    let second = if parts.len() == 3 {
        parts[2].parse::<u8>().ok()
    } else {
        Some(0)
    };
    if !matches!(
        (hour, minute, second),
        (Some(hour), Some(minute), Some(second))
            if hour <= 23 && minute <= 59 && second <= 59
    ) {
        errors.push(FieldError {
            path: path.to_owned(),
            message: "must be a valid local time with hour 0..=23, minute 0..=59, second 0..=59"
                .to_owned(),
        });
        return None;
    }
    Some(LocalTime {
        hour: hour.expect("validated hour"),
        minute: minute.expect("validated minute"),
        second: second.expect("validated second"),
    })
}

/// Parses a compound duration such as `1h30m`, `60s`, or `0s`. When `signed`
/// is set a leading `-` is accepted and the result may be negative. Units must
/// appear in a canonical order without repetition (`1h30m`, never `30m1h`).
pub(crate) fn parse_duration_seconds(value: &str, signed: bool) -> Result<i64, ()> {
    let (negative, rest) = match value.strip_prefix('-') {
        Some(rest) if signed => (true, rest),
        Some(_) => return Err(()),
        None => (false, value),
    };
    if rest.is_empty() {
        return Err(());
    }
    let mut total: i64 = 0;
    let mut digits = String::new();
    let mut last_unit: Option<char> = None;
    let mut last_unit_rank = 0u8;
    for character in rest.chars() {
        if character.is_ascii_digit() {
            digits.push(character);
        } else {
            let count: i64 = digits.parse().map_err(|_| ())?;
            digits.clear();
            let unit_seconds = match character {
                'h' => 3_600,
                'm' => 60,
                's' => 1,
                _ => return Err(()),
            };
            if last_unit == Some(character) {
                return Err(());
            }
            let unit_rank = match character {
                'h' => 3,
                'm' => 2,
                's' => 1,
                _ => unreachable!("duration unit validated above"),
            };
            if last_unit.is_some() && unit_rank >= last_unit_rank {
                return Err(());
            }
            last_unit = Some(character);
            last_unit_rank = unit_rank;
            total = total
                .checked_add(count.checked_mul(unit_seconds).ok_or(())?)
                .ok_or(())?;
        }
    }
    if last_unit.is_none() || !digits.is_empty() {
        return Err(());
    }
    Ok(if negative { -total } else { total })
}

fn parse_duration_field(
    path: &str,
    value: &str,
    signed: bool,
    errors: &mut Vec<FieldError>,
) -> Option<i64> {
    match parse_duration_seconds(value, signed) {
        Ok(seconds) => Some(seconds),
        Err(()) => {
            errors.push(FieldError {
                path: path.to_owned(),
                message: if signed {
                    "must be a signed whole-second duration such as -1h30m, 30m, or 45s".to_owned()
                } else {
                    "must be a whole-second duration such as 1h30m, 60s, or 0s".to_owned()
                },
            });
            None
        }
    }
}

fn weekday_ordinal(weekday: Weekday) -> u8 {
    match weekday {
        Weekday::Monday => 0,
        Weekday::Tuesday => 1,
        Weekday::Wednesday => 2,
        Weekday::Thursday => 3,
        Weekday::Friday => 4,
        Weekday::Saturday => 5,
        Weekday::Sunday => 6,
    }
}

/// Parses weekday tokens into a unique, calendar-ordered set.
fn parse_weekdays(
    path: &str,
    tokens: &[String],
    errors: &mut Vec<FieldError>,
) -> Option<Vec<Weekday>> {
    if tokens.is_empty() {
        errors.push(FieldError {
            path: path.to_owned(),
            message: "must not be empty; an empty weekday list can never produce an occurrence"
                .to_owned(),
        });
        return None;
    }
    let mut seen = HashSet::new();
    let mut days = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let weekday = match token.as_str() {
            "mon" => Some(Weekday::Monday),
            "tue" => Some(Weekday::Tuesday),
            "wed" => Some(Weekday::Wednesday),
            "thu" => Some(Weekday::Thursday),
            "fri" => Some(Weekday::Friday),
            "sat" => Some(Weekday::Saturday),
            "sun" => Some(Weekday::Sunday),
            _ => None,
        };
        let Some(weekday) = weekday else {
            errors.push(FieldError {
                path: format!("{path}[{index}]"),
                message: "must be one of mon, tue, wed, thu, fri, sat, or sun".to_owned(),
            });
            continue;
        };
        if !seen.insert(weekday) {
            errors.push(FieldError {
                path: format!("{path}[{index}]"),
                message: "must not repeat a weekday".to_owned(),
            });
            continue;
        }
        days.push(weekday);
    }
    days.sort_by_key(|weekday| weekday_ordinal(*weekday));
    Some(days)
}

fn parse_anchor(path: &str, value: &str, errors: &mut Vec<FieldError>) -> Option<SolarAnchor> {
    match value {
        "dawn" => Some(SolarAnchor::Dawn),
        "sunrise" => Some(SolarAnchor::Sunrise),
        "sunset" => Some(SolarAnchor::Sunset),
        "dusk" => Some(SolarAnchor::Dusk),
        _ => {
            errors.push(FieldError {
                path: path.to_owned(),
                message: "must be one of dawn, sunrise, sunset, or dusk".to_owned(),
            });
            None
        }
    }
}

/// Rejects kind-specific fields that the selected schedule kind does not own.
fn reject_disallowed_fields(
    schedule: &AutomationSchedule,
    path: &str,
    allowed: &[&str],
    errors: &mut Vec<FieldError>,
) {
    for (field, present) in [
        ("at", schedule.at.is_some()),
        ("every", schedule.every.is_some()),
        ("offset", schedule.offset.is_some()),
        ("anchor", schedule.anchor.is_some()),
        ("weekdays", schedule.weekdays.is_some()),
    ] {
        if present && !allowed.contains(&field) {
            errors.push(FieldError {
                path: format!("{path}.{field}"),
                message: format!("is not allowed for a {kind} schedule", kind = schedule.kind),
            });
        }
    }
}

/// Validates one document schedule and converts it into the portable core
/// rule. All validation errors are collected; `None` is returned only when
/// the rule cannot be constructed.
pub(crate) fn schedule_rule(
    schedule: &AutomationSchedule,
    path: &str,
    errors: &mut Vec<FieldError>,
) -> Option<ScheduleRule> {
    for field in schedule.extra.keys() {
        errors.push(FieldError {
            path: format!("{path}.{field}"),
            message: "unknown field for schedule".to_owned(),
        });
    }
    match schedule.kind.as_str() {
        "fixed" => {
            reject_disallowed_fields(schedule, path, &["at", "weekdays"], errors);
            let at = match schedule.at.as_deref() {
                Some(value) => parse_local_time(&format!("{path}.at"), value, errors),
                None => {
                    errors.push(FieldError {
                        path: format!("{path}.at"),
                        message: "is required for a fixed schedule".to_owned(),
                    });
                    None
                }
            };
            let weekdays = match schedule.weekdays.as_deref() {
                Some(tokens) => parse_weekdays(&format!("{path}.weekdays"), tokens, errors),
                None => None,
            };
            // Omitted weekdays mean every day; the core set is non-empty, so
            // every day is encoded as the full calendar week.
            let weekdays = match weekdays {
                Some(days) => days,
                None => Weekday::ALL.to_vec(),
            };
            let weekdays = match WeekdaySet::new(&weekdays) {
                Ok(set) => set,
                Err(error) => {
                    errors.push(FieldError {
                        path: format!("{path}.weekdays"),
                        message: error.to_string(),
                    });
                    return None;
                }
            };
            Some(ScheduleRule::Fixed { at: at?, weekdays })
        }
        "interval" => {
            reject_disallowed_fields(schedule, path, &["every", "offset"], errors);
            let every = match schedule.every.as_deref() {
                Some(value) => parse_duration_field(&format!("{path}.every"), value, false, errors),
                None => {
                    errors.push(FieldError {
                        path: format!("{path}.every"),
                        message: "is required for an interval schedule".to_owned(),
                    });
                    None
                }
            };
            let offset = match schedule.offset.as_deref() {
                Some(value) => {
                    parse_duration_field(&format!("{path}.offset"), value, false, errors)
                }
                None => Some(0),
            };
            let (Some(every), Some(offset)) = (every, offset) else {
                return None;
            };
            if !(60..=604_800).contains(&every) {
                errors.push(FieldError {
                    path: format!("{path}.every"),
                    message: "must be between 60s and 7 days (604800s)".to_owned(),
                });
                return None;
            }
            if !(0..every).contains(&offset) {
                errors.push(FieldError {
                    path: format!("{path}.offset"),
                    message: format!("must be between 0s and every - 1s ({every}s)"),
                });
                return None;
            }
            Some(ScheduleRule::Interval {
                every_seconds: every as u32,
                offset_seconds: offset as u32,
            })
        }
        "astronomical" => {
            reject_disallowed_fields(schedule, path, &["anchor", "offset", "weekdays"], errors);
            let anchor = match schedule.anchor.as_deref() {
                Some(value) => parse_anchor(&format!("{path}.anchor"), value, errors),
                None => {
                    errors.push(FieldError {
                        path: format!("{path}.anchor"),
                        message: "is required for an astronomical schedule".to_owned(),
                    });
                    None
                }
            };
            let offset = match schedule.offset.as_deref() {
                Some(value) => parse_duration_field(&format!("{path}.offset"), value, true, errors),
                None => {
                    errors.push(FieldError {
                        path: format!("{path}.offset"),
                        message: "is required for an astronomical schedule".to_owned(),
                    });
                    None
                }
            };
            let weekdays = match schedule.weekdays.as_deref() {
                Some(tokens) => parse_weekdays(&format!("{path}.weekdays"), tokens, errors),
                None => None,
            };
            let (Some(anchor), Some(offset)) = (anchor, offset) else {
                return None;
            };
            if !(-86_400..=86_400).contains(&offset) {
                errors.push(FieldError {
                    path: format!("{path}.offset"),
                    message: "must be between -24h (-86400s) and +24h (86400s)".to_owned(),
                });
                return None;
            }
            let weekdays = match weekdays {
                Some(days) => days,
                None => Weekday::ALL.to_vec(),
            };
            let weekdays = match WeekdaySet::new(&weekdays) {
                Ok(set) => set,
                Err(error) => {
                    errors.push(FieldError {
                        path: format!("{path}.weekdays"),
                        message: error.to_string(),
                    });
                    return None;
                }
            };
            Some(ScheduleRule::Astronomical {
                anchor,
                offset_seconds: offset as i32,
                weekdays,
            })
        }
        other => {
            errors.push(FieldError {
                path: format!("{path}.kind"),
                message: format!("must be one of fixed, interval, or astronomical, not {other:?}"),
            });
            None
        }
    }
}

/// Validates a block's schedule list and converts it into core block
/// schedules. Errors are appended to `errors` with precise paths.
fn block_schedules(
    schedules: &[AutomationSchedule],
    block_path: &str,
    errors: &mut Vec<FieldError>,
) -> Vec<CoreBlockSchedule> {
    if schedules.len() > MAX_SCHEDULES_PER_BLOCK {
        errors.push(FieldError {
            path: format!("{block_path}.schedules"),
            message: format!("must contain at most {MAX_SCHEDULES_PER_BLOCK} schedules"),
        });
    }
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for (index, schedule) in schedules.iter().enumerate() {
        let path = format!("{block_path}.schedules[{index}]");
        let name = match ScheduleName::new(&schedule.name) {
            Ok(name) => name,
            Err(error) => {
                errors.push(FieldError {
                    path: format!("{path}.name"),
                    message: error.to_string(),
                });
                continue;
            }
        };
        if !seen.insert(name.clone()) {
            errors.push(FieldError {
                path: format!("{path}.name"),
                message: "must be unique within this block".to_owned(),
            });
        }
        if let Some(rule) = schedule_rule(schedule, &path, errors) {
            result.push(CoreBlockSchedule {
                name,
                enabled: schedule.enabled,
                rule,
            });
        }
    }
    result
}
