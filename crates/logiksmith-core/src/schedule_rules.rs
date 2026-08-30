use std::{error::Error, fmt, str::FromStr};

use jiff::civil::{Date, DateTime, Weekday as JiffWeekday};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, Zoned};

use mlua::{AnyUserData, IntoLua, UserData, UserDataFields, UserDataMethods, Value as LuaValue};

use crate::noaa;
use crate::{BlockId, EndpointNameError, LogicError, LogicRevision, MonotonicMs, SimulationError};

/// A UTC-unix-millisecond instant (milliseconds since the Unix epoch).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct UtcUnixMs(pub i64);

/// One host clock sample: a monotonic timestamp plus the wall-clock UTC
/// instant when known. `utc_unix_ms: None` means the wall clock is invalid or
/// not yet available; schedules pause and captured time contexts are
/// unavailable sentinels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockSample {
    pub monotonic_ms: MonotonicMs,
    pub utc_unix_ms: Option<i64>,
}

/// Geographic coordinates: latitude/longitude in decimal degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coordinates {
    pub latitude: f64,
    pub longitude: f64,
}

/// A validated IANA time zone identifier (for example `Europe/Berlin`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeZoneId(String);

/// A time zone identifier that could not be found in the bundled IANA
/// time zone database.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeZoneIdError(String);

impl TimeZoneId {
    /// Validates `value` against the bundled IANA time zone database.
    pub fn new(value: &str) -> Result<Self, TimeZoneIdError> {
        TimeZone::get(value).map_err(|_| TimeZoneIdError(value.to_owned()))?;
        Ok(Self(value.to_owned()))
    }
    /// The fixed `UTC` identifier.
    pub fn utc() -> Self {
        Self::new("UTC").expect("UTC is a valid IANA time zone identifier")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TimeZoneId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl fmt::Display for TimeZoneIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown IANA time zone identifier {:?}", self.0)
    }
}

impl Error for TimeZoneIdError {}

/// The site configuration a runtime captures time from: the local time zone
/// and, when available, the coordinates used for solar events.
#[derive(Clone, Debug, PartialEq)]
pub struct SiteTimeConfig {
    pub timezone: TimeZoneId,
    pub coordinates: Option<Coordinates>,
}

/// A validated schedule name. Schedule names use the same lexical grammar as
/// endpoint names: a lowercase ASCII letter followed by lowercase ASCII
/// letters, digits, `_`, `-`, or `.`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ScheduleName(String);

pub type ScheduleNameError = EndpointNameError;

impl ScheduleName {
    pub fn new(value: impl Into<String>) -> Result<Self, ScheduleNameError> {
        let value = value.into();
        crate::validate_endpoint_name(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ScheduleName {
    type Err = ScheduleNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl fmt::Display for ScheduleName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A local wall-clock time of day. Bounds are validated by
/// [`ScheduleRule::validate`] (via `BlockConfig::validate`).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LocalTime {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// A day of the week. `Display` writes the full English name (`Monday`..`Sunday`),
/// matching the string scripts read from `ctx.now.weekday`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    /// All weekdays in calendar order.
    pub const ALL: [Weekday; 7] = [
        Weekday::Monday,
        Weekday::Tuesday,
        Weekday::Wednesday,
        Weekday::Thursday,
        Weekday::Friday,
        Weekday::Saturday,
        Weekday::Sunday,
    ];
}

impl fmt::Display for Weekday {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Weekday::Monday => "Monday",
            Weekday::Tuesday => "Tuesday",
            Weekday::Wednesday => "Wednesday",
            Weekday::Thursday => "Thursday",
            Weekday::Friday => "Friday",
            Weekday::Saturday => "Saturday",
            Weekday::Sunday => "Sunday",
        })
    }
}

/// An error validating a [`WeekdaySet`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WeekdaySetError {
    Empty,
    Duplicate(Weekday),
}

impl fmt::Display for WeekdaySetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("weekday set must not be empty"),
            Self::Duplicate(weekday) => write!(formatter, "duplicate weekday {weekday}"),
        }
    }
}

impl Error for WeekdaySetError {}

/// A non-empty set of weekdays, stored in calendar order for deterministic
/// display.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WeekdaySet(Vec<Weekday>);

impl WeekdaySet {
    /// Builds a set from `weekdays`, rejecting empty or duplicate input.
    pub fn new(weekdays: &[Weekday]) -> Result<Self, WeekdaySetError> {
        if weekdays.is_empty() {
            return Err(WeekdaySetError::Empty);
        }
        let mut sorted = weekdays.to_vec();
        for (index, weekday) in sorted.iter().enumerate() {
            if sorted.iter().take(index).any(|other| other == weekday) {
                return Err(WeekdaySetError::Duplicate(*weekday));
            }
        }
        sorted.sort_unstable();
        Ok(Self(sorted))
    }

    pub fn contains(&self, weekday: Weekday) -> bool {
        self.0.contains(&weekday)
    }
}

impl fmt::Display for WeekdaySet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, weekday) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{weekday}")?;
        }
        Ok(())
    }
}

/// The solar event a [`ScheduleRule::Astronomical`] rule anchors on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SolarAnchor {
    Dawn,
    Sunrise,
    Sunset,
    Dusk,
}

/// One schedule rule. See the module documentation for exact semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleRule {
    Fixed {
        at: LocalTime,
        weekdays: WeekdaySet,
    },
    Interval {
        every_seconds: u32,
        offset_seconds: u32,
    },
    Astronomical {
        anchor: SolarAnchor,
        offset_seconds: i32,
        weekdays: WeekdaySet,
    },
}

impl ScheduleRule {
    pub fn kind(&self) -> ScheduleKind {
        match self {
            ScheduleRule::Fixed { .. } => ScheduleKind::Fixed,
            ScheduleRule::Interval { .. } => ScheduleKind::Interval,
            ScheduleRule::Astronomical { .. } => ScheduleKind::Astronomical,
        }
    }

    /// Validates the numeric bounds every rule carries. Called by
    /// `BlockConfig::validate`; the engine additionally guards against
    /// out-of-range values so it never panics even on unvalidated input.
    pub(crate) fn validate(&self) -> Result<(), ScheduleError> {
        match self {
            ScheduleRule::Fixed { at, .. } => validate_local_time(at),
            ScheduleRule::Interval {
                every_seconds,
                offset_seconds,
            } => {
                if !(MIN_INTERVAL_SECONDS..=MAX_INTERVAL_SECONDS).contains(every_seconds) {
                    return Err(ScheduleError::InvalidInterval {
                        every_seconds: *every_seconds,
                    });
                }
                if *offset_seconds >= *every_seconds {
                    return Err(ScheduleError::InvalidIntervalOffset {
                        offset_seconds: *offset_seconds,
                        every_seconds: *every_seconds,
                    });
                }
                Ok(())
            }
            ScheduleRule::Astronomical { offset_seconds, .. } => {
                if !(-MAX_ASTRONOMICAL_OFFSET_SECONDS..=MAX_ASTRONOMICAL_OFFSET_SECONDS)
                    .contains(offset_seconds)
                {
                    return Err(ScheduleError::InvalidAstronomicalOffset {
                        offset_seconds: *offset_seconds,
                    });
                }
                Ok(())
            }
        }
    }
}

fn validate_local_time(at: &LocalTime) -> Result<(), ScheduleError> {
    if at.hour > 23 || at.minute > 59 || at.second > 59 {
        return Err(ScheduleError::InvalidLocalTime(*at));
    }
    Ok(())
}

fn is_valid_local_time(at: &LocalTime) -> bool {
    at.hour <= 23 && at.minute <= 59 && at.second <= 59
}

/// One configured schedule inside a logic block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockSchedule {
    pub name: ScheduleName,
    pub enabled: bool,
    pub rule: ScheduleRule,
}

/// The family of a schedule rule, mirrored on every delivered
/// [`ScheduleTrigger`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleKind {
    Fixed,
    Interval,
    Astronomical,
}

impl fmt::Display for ScheduleKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            ScheduleKind::Fixed => "fixed",
            ScheduleKind::Interval => "interval",
            ScheduleKind::Astronomical => "astronomical",
        })
    }
}

/// One occurrence of a schedule, as returned by
/// `Runtime::preview_occurrences`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleOccurrence {
    pub utc_ms: i64,
}

/// A due schedule occurrence delivered by `Runtime::poll_schedules`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleTrigger {
    pub block_id: BlockId,
    pub name: ScheduleName,
    pub kind: ScheduleKind,
    pub scheduled_for_utc_ms: i64,
    pub detected_at_utc_ms: i64,
    /// Occurrences that passed between polls and were folded into this
    /// delivery (latest-only coalescing).
    pub coalesced_count: u64,
    pub structural_revision: u64,
}

/// A comparable civil date-time value exposed to Lua as `ctx.now` and the
/// `ctx.sun` events. The `instant` is private: comparison uses it, scripts
/// cannot read it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DateTimeValue {
    pub available: bool,
    pub year: Option<i32>,
    pub month: Option<u8>,
    pub day: Option<u8>,
    pub hour: Option<u8>,
    pub minute: Option<u8>,
    pub second: Option<u8>,
    pub weekday: Option<Weekday>,
    instant: Option<i64>,
}

/// The solar context captured with a [`TimeContext`].
#[derive(Clone, Debug, PartialEq)]
pub struct SunContext {
    pub dawn: DateTimeValue,
    pub sunrise: DateTimeValue,
    pub sunset: DateTimeValue,
    pub dusk: DateTimeValue,
    pub elevation_degrees: Option<f64>,
    pub azimuth_degrees: Option<f64>,
}

/// The frozen wall-clock context handed to Lua as the fifth `ctx` argument.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeContext {
    pub now: DateTimeValue,
    pub sun: SunContext,
}

impl TimeContext {
    /// Captures the frozen time context for a wall-clock instant (UTC ms) and
    /// site. `None` utc (or a site with no time zone resolution) yields `now`
    /// unavailable and every sun event an unavailable sentinel.
    pub fn capture(site: &SiteTimeConfig, utc_unix_ms: Option<i64>) -> TimeContext {
        let Some(utc) = utc_unix_ms else {
            return TimeContext::unavailable();
        };
        let Ok(tz) = TimeZone::get(site.timezone.as_str()) else {
            return TimeContext::unavailable();
        };
        let Some(now) = local_datetime_of(&tz, utc) else {
            return TimeContext::unavailable();
        };
        let now_value = DateTimeValue {
            available: true,
            year: Some(i32::from(now.year())),
            month: Some(now.month() as u8),
            day: Some(now.day() as u8),
            hour: Some(now.hour() as u8),
            minute: Some(now.minute() as u8),
            second: Some(now.second() as u8),
            weekday: Some(weekday_of(now.date())),
            instant: Some(utc),
        };
        let sun = match site.coordinates {
            Some(coordinates) => {
                let position =
                    noaa::solar_position_utc(utc, coordinates.latitude, coordinates.longitude);
                SunContext {
                    dawn: event_value(&tz, now.date(), coordinates, DAWN_THRESHOLD_DEGREES, false),
                    sunrise: event_value(
                        &tz,
                        now.date(),
                        coordinates,
                        SUNRISE_THRESHOLD_DEGREES,
                        false,
                    ),
                    sunset: event_value(
                        &tz,
                        now.date(),
                        coordinates,
                        SUNRISE_THRESHOLD_DEGREES,
                        true,
                    ),
                    dusk: event_value(&tz, now.date(), coordinates, DAWN_THRESHOLD_DEGREES, true),
                    elevation_degrees: Some(position.elevation_degrees),
                    azimuth_degrees: Some(position.azimuth_degrees),
                }
            }
            None => SunContext::unavailable(),
        };
        TimeContext {
            now: now_value,
            sun,
        }
    }

    fn unavailable() -> TimeContext {
        TimeContext {
            now: DateTimeValue::unavailable(),
            sun: SunContext::unavailable(),
        }
    }
}

impl DateTimeValue {
    fn unavailable() -> DateTimeValue {
        DateTimeValue {
            available: false,
            year: None,
            month: None,
            day: None,
            hour: None,
            minute: None,
            second: None,
            weekday: None,
            instant: None,
        }
    }
}

impl SunContext {
    fn unavailable() -> SunContext {
        SunContext {
            dawn: DateTimeValue::unavailable(),
            sunrise: DateTimeValue::unavailable(),
            sunset: DateTimeValue::unavailable(),
            dusk: DateTimeValue::unavailable(),
            elevation_degrees: None,
            azimuth_degrees: None,
        }
    }
}

/// Lua exposure of [`DateTimeValue`]: civil fields via `__index`, ordering and
/// equality via the hidden instant. Ordering against a string compares the
/// local time of day. Unavailable values expose `nil` fields and compare
/// `false` against everything, including themselves.
impl UserData for DateTimeValue {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_function_get("year", |_, ud| {
            let value = ud.borrow::<DateTimeValue>()?;
            Ok(value.year.map(i64::from))
        });
        fields.add_field_function_get("month", |_, ud| {
            let value = ud.borrow::<DateTimeValue>()?;
            Ok(value.month.map(i64::from))
        });
        fields.add_field_function_get("day", |_, ud| {
            let value = ud.borrow::<DateTimeValue>()?;
            Ok(value.day.map(i64::from))
        });
        fields.add_field_function_get("hour", |_, ud| {
            let value = ud.borrow::<DateTimeValue>()?;
            Ok(value.hour.map(i64::from))
        });
        fields.add_field_function_get("minute", |_, ud| {
            let value = ud.borrow::<DateTimeValue>()?;
            Ok(value.minute.map(i64::from))
        });
        fields.add_field_function_get("second", |_, ud| {
            let value = ud.borrow::<DateTimeValue>()?;
            Ok(value.second.map(i64::from))
        });
        fields.add_field_function_get("weekday", |lua, ud| {
            let value = ud.borrow::<DateTimeValue>()?;
            Ok(value
                .weekday
                .map(|weekday| weekday.to_string())
                .into_lua(lua)?)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_function("__eq", |_, (left, right): (AnyUserData, AnyUserData)| {
            Ok(compare_datetime_values(&left, &right, |a, b| a == b))
        });
        methods.add_meta_function("__lt", |_, (left, right): (AnyUserData, LuaValue)| {
            compare_datetime_value_to_lua(&left, right, |a, b| a < b)
        });
        methods.add_meta_function("__le", |_, (left, right): (AnyUserData, LuaValue)| {
            compare_datetime_value_to_lua(&left, right, |a, b| a <= b)
        });
    }
}

fn compare_datetime_value_to_lua<F>(
    left: &AnyUserData,
    right: LuaValue,
    compare: F,
) -> mlua::Result<bool>
where
    F: Fn(i64, i64) -> bool,
{
    match right {
        LuaValue::UserData(right) => Ok(compare_datetime_values(left, &right, compare)),
        LuaValue::String(right) => {
            let right = parse_local_time_string(&right)?;
            let Ok(left) = left.borrow::<DateTimeValue>() else {
                return Ok(false);
            };
            let Some(left) = local_seconds(&left) else {
                return Ok(false);
            };
            Ok(compare(left, right))
        }
        _ => Ok(false),
    }
}

fn parse_local_time_string(value: &mlua::String) -> mlua::Result<i64> {
    let bytes = value.as_bytes();
    let fields = match bytes.len() {
        5 => [
            parse_two_digits(&bytes[0..2]),
            parse_two_digits(&bytes[3..5]),
            Some(0),
        ],
        8 => [
            parse_two_digits(&bytes[0..2]),
            parse_two_digits(&bytes[3..5]),
            parse_two_digits(&bytes[6..8]),
        ],
        _ => [None, None, None],
    };
    let valid_separators = bytes.get(2) == Some(&b':')
        && (bytes.len() == 5 || bytes.get(5) == Some(&b':'));
    if !valid_separators
        || fields.iter().any(Option::is_none)
        || fields[0].is_some_and(|hour| hour > 23)
        || fields[1].is_some_and(|minute| minute > 59)
        || fields[2].is_some_and(|second| second > 59)
    {
        return Err(mlua::Error::RuntimeError(format!(
            "invalid local time {:?}: expected canonical HH:MM or HH:MM:SS",
            value.to_string_lossy()
        )));
    }
    Ok(fields[0].unwrap_or_default() * 3_600
        + fields[1].unwrap_or_default() * 60
        + fields[2].unwrap_or_default())
}

fn parse_two_digits(value: &[u8]) -> Option<i64> {
    (value.len() == 2
        && value[0].is_ascii_digit()
        && value[1].is_ascii_digit())
        .then(|| i64::from(value[0] - b'0') * 10 + i64::from(value[1] - b'0'))
}

fn local_seconds(value: &DateTimeValue) -> Option<i64> {
    if !value.available {
        return None;
    }
    Some(
        i64::from(value.hour?) * 3_600
            + i64::from(value.minute?) * 60
            + i64::from(value.second?),
    )
}

fn compare_datetime_values<F>(left: &AnyUserData, right: &AnyUserData, compare: F) -> bool
where
    F: Fn(i64, i64) -> bool,
{
    let Ok(left) = left.borrow::<DateTimeValue>() else {
        return false;
    };
    let Ok(right) = right.borrow::<DateTimeValue>() else {
        return false;
    };
    match (left.instant, right.instant) {
        (Some(a), Some(b)) => compare(a, b),
        _ => false,
    }
}

/// A request to simulate one schedule occurrence without mutating the runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleSimulationRequest {
    pub block_id: BlockId,
    pub expected_logic_revision: LogicRevision,
    pub expected_structural_revision: u64,
    pub schedule: ScheduleName,
    pub occurrence_at_utc_ms: i64,
}

/// Errors from [`ScheduleSimulationRequest`] validation. Lua failures are
/// contained in `Execution::outcome`, matching input/timer simulations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleSimulationError {
    UnknownSchedule,
    NotOccurrence,
    StaleStructuralRevision,
    InvalidSource(LogicError),
    InvalidInput(SimulationError),
}

impl fmt::Display for ScheduleSimulationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSchedule => formatter.write_str("unknown schedule or block"),
            Self::NotOccurrence => {
                formatter.write_str("the requested instant is not an occurrence of the schedule")
            }
            Self::StaleStructuralRevision => formatter.write_str(
                "the requested revisions do not match the current schedule/logic revision",
            ),
            Self::InvalidSource(error) => write!(formatter, "invalid simulation source: {error}"),
            Self::InvalidInput(error) => error.fmt(formatter),
        }
    }
}

impl Error for ScheduleSimulationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidSource(error) => Some(error),
            Self::InvalidInput(error) => Some(error),
            _ => None,
        }
    }
}

/// The status of one configured schedule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleStatusKind {
    Active,
    Paused,
    Unavailable { reason: String },
    ClockError,
}

/// A public per-schedule status view produced by `Runtime::schedule_statuses`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleStatus {
    pub block_id: BlockId,
    pub name: ScheduleName,
    pub enabled: bool,
    pub status: ScheduleStatusKind,
    pub next_occurrence_utc_ms: Option<i64>,
}

/// Errors from the wall-clock schedule engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimeError {
    /// A UTC wall-clock instant is required but none was supplied.
    ClockUnavailable,
    /// A host supplied a monotonic sample earlier than one already accepted
    /// by the serial runtime. Schedules must use the same monotonic ordering
    /// invariant as input and named-timer events.
    MonotonicWentBackwards {
        previous: MonotonicMs,
        current: MonotonicMs,
    },
    /// A host asked to rebaseline a block that is not part of this runtime.
    UnknownBlock(BlockId),
    /// An internal invariant was violated (configuration validation should
    /// have prevented this).
    Invariant(String),
}

impl fmt::Display for TimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClockUnavailable => formatter.write_str(
                "schedule engine requires a UTC wall-clock instant, but none was supplied",
            ),
            Self::MonotonicWentBackwards { previous, current } => write!(
                formatter,
                "schedule sample time {current:?} is earlier than the last accepted time {previous:?}"
            ),
            Self::UnknownBlock(id) => write!(formatter, "unknown logic block {id}"),
            Self::Invariant(message) => {
                write!(formatter, "schedule engine invariant violated: {message}")
            }
        }
    }
}

impl Error for TimeError {}

/// Errors from schedule configuration or lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleError {
    UnknownSchedule,
    InvalidLocalTime(LocalTime),
    InvalidInterval {
        every_seconds: u32,
    },
    InvalidIntervalOffset {
        offset_seconds: u32,
        every_seconds: u32,
    },
    InvalidAstronomicalOffset {
        offset_seconds: i32,
    },
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSchedule => formatter.write_str("unknown schedule"),
            Self::InvalidLocalTime(at) => {
                write!(
                    formatter,
                    "invalid local time {:02}:{:02}:{:02}",
                    at.hour, at.minute, at.second
                )
            }
            Self::InvalidInterval { every_seconds } => write!(
                formatter,
                "interval every_seconds must be within {MIN_INTERVAL_SECONDS}..={MAX_INTERVAL_SECONDS}, got {every_seconds}"
            ),
            Self::InvalidIntervalOffset {
                offset_seconds,
                every_seconds,
            } => write!(
                formatter,
                "interval offset_seconds must be below every_seconds ({every_seconds}), got {offset_seconds}"
            ),
            Self::InvalidAstronomicalOffset { offset_seconds } => write!(
                formatter,
                "astronomical offset_seconds must be within -{MAX_ASTRONOMICAL_OFFSET_SECONDS}..={MAX_ASTRONOMICAL_OFFSET_SECONDS}, got {offset_seconds}"
            ),
        }
    }
}

impl Error for ScheduleError {}

// --- bounds -----------------------------------------------------------------

/// Maximum number of schedules per logic block.
pub const MAX_SCHEDULES_PER_BLOCK: usize = 32;
/// Minimum `every_seconds` for interval rules.
pub const MIN_INTERVAL_SECONDS: u32 = 60;
/// Maximum `every_seconds` for interval rules.
pub const MAX_INTERVAL_SECONDS: u32 = 604_800;
/// Maximum absolute `offset_seconds` for astronomical rules.
pub const MAX_ASTRONOMICAL_OFFSET_SECONDS: i32 = 86_400;

/// How many days a search walks before declaring an astronomical schedule
/// unavailable (covers a full polar year cycle).
pub(crate) const SEARCH_DAY_LIMIT: i64 = 370;

/// Apparent-horizon threshold for sunrise/sunset (solar centre).
const SUNRISE_THRESHOLD_DEGREES: f64 = -0.833;
/// Civil twilight threshold for dawn/dusk.
const DAWN_THRESHOLD_DEGREES: f64 = -6.0;

// --- engine state ------------------------------------------------------------

/// Per-block, per-schedule engine state. Private to the crate; owned by
/// `Runtime`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScheduleCursor {
    pub(crate) last_delivered_utc_ms: Option<i64>,
    pub(crate) next_occurrence_utc_ms: Option<i64>,
    pub(crate) structural_revision: u64,
    /// Set when a cursor needs a future-only baseline before it can be
    /// considered for delivery. This covers an invalid startup clock and
    /// block re-enable without confusing a polar unavailable schedule with a
    /// schedule that has not yet been sampled.
    pub(crate) needs_rebaseline: bool,
}

// --- occurrence engine -------------------------------------------------------

/// The first occurrence of `rule` strictly after `after_utc_ms`, or `None`
/// when the search window is exhausted (astronomical polar schedules) or the
/// rule cannot fire (no site coordinates).
///
/// The result is deterministic: same rule, site, and baseline always produce
/// the same occurrence.
pub(crate) fn next_occurrence_after(
    rule: &ScheduleRule,
    site: &SiteTimeConfig,
    after_utc_ms: i64,
) -> Option<i64> {
    match rule {
        ScheduleRule::Fixed { at, weekdays } => {
            if !is_valid_local_time(at) {
                return None;
            }
            let tz = TimeZone::get(site.timezone.as_str()).ok()?;
            let baseline_date = local_date_of_utc(&tz, after_utc_ms)?;
            for day in 0..SEARCH_DAY_LIMIT {
                let anchor = baseline_date.checked_add(Span::new().days(day)).ok()?;
                if !weekdays.contains(weekday_of(anchor)) {
                    continue;
                }
                if let Some(candidate) = resolve_local(
                    anchor.at(at.hour as i8, at.minute as i8, at.second as i8, 0),
                    &tz,
                ) && candidate > after_utc_ms
                {
                    return Some(candidate);
                }
            }
            // Unreachable for a validated rule (weekdays are non-empty), but
            // kept total so the engine never loops or panics.
            None
        }
        ScheduleRule::Interval {
            every_seconds,
            offset_seconds,
        } => {
            let every = i64::from(*every_seconds);
            if every == 0 {
                return None;
            }
            let offset = i64::from(*offset_seconds);
            let baseline_seconds = after_utc_ms.div_euclid(1000);
            let numerator = baseline_seconds + 1 - offset;
            let k = numerator.div_euclid(every) + i64::from(numerator.rem_euclid(every) != 0);
            Some((offset + k * every).saturating_mul(1000))
        }
        ScheduleRule::Astronomical {
            anchor,
            offset_seconds,
            weekdays,
        } => {
            let coordinates = site.coordinates?;
            let tz = TimeZone::get(site.timezone.as_str()).ok()?;
            let baseline_date = local_date_of_utc(&tz, after_utc_ms)?;
            let mut best: Option<i64> = None;
            // Anchors start one day before the baseline date: a signed offset
            // on the previous day's event can land after the baseline. We
            // collect every candidate in the window and keep the earliest so
            // delivery order stays monotonic even when an offset/guard pushes
            // an occurrence across a date boundary.
            // ponytail: the -1..370 anchor window covers every possible
            // occurrence a ±24h offset and same-day guards can produce; a
            // wider spread would need multi-day offsets, which validation
            // already rejects.
            for day in -1..SEARCH_DAY_LIMIT {
                let anchor_date = baseline_date.checked_add(Span::new().days(day)).ok()?;
                let Some(candidate) = astronomical_occurrence(
                    anchor_date,
                    *anchor,
                    *offset_seconds,
                    weekdays,
                    &tz,
                    coordinates,
                ) else {
                    continue;
                };
                if candidate > after_utc_ms && best.is_none_or(|current| candidate < current) {
                    best = Some(candidate);
                }
            }
            best
        }
    }
}
