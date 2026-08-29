fn astronomical_occurrence(
    anchor: Date,
    anchor_kind: SolarAnchor,
    offset_seconds: i32,
    earliest: Option<LocalTime>,
    latest: Option<LocalTime>,
    weekdays: &WeekdaySet,
    tz: &TimeZone,
    coordinates: Coordinates,
) -> Option<i64> {
    let (threshold, evening) = match anchor_kind {
        SolarAnchor::Dawn => (DAWN_THRESHOLD_DEGREES, false),
        SolarAnchor::Sunrise => (SUNRISE_THRESHOLD_DEGREES, false),
        SolarAnchor::Sunset => (SUNRISE_THRESHOLD_DEGREES, true),
        SolarAnchor::Dusk => (DAWN_THRESHOLD_DEGREES, true),
    };
    let event_utc = solar_event_for_local_date(
        tz,
        anchor,
        coordinates.latitude,
        coordinates.longitude,
        threshold,
        evening,
    )?;
    let candidate_utc = event_utc.saturating_add(i64::from(offset_seconds) * 1000);
    let local = local_datetime_of(tz, candidate_utc)?;
    let mut final_dt = local;
    if let Some(earliest_time) = earliest
        && is_valid_local_time(&earliest_time)
        && local_time_of(local) < earliest_time
    {
        final_dt = local.date().at(
            earliest_time.hour as i8,
            earliest_time.minute as i8,
            earliest_time.second as i8,
            0,
        );
    }
    if let Some(latest_time) = latest
        && is_valid_local_time(&latest_time)
        && local_time_of(final_dt) > latest_time
    {
        final_dt = local.date().at(
            latest_time.hour as i8,
            latest_time.minute as i8,
            latest_time.second as i8,
            0,
        );
    }
    // Weekday filter applies to the FINAL computed occurrence's local date.
    if !weekdays.contains(weekday_of(final_dt.date())) {
        return None;
    }
    resolve_local(final_dt, tz)
}
/// UTC milliseconds of the solar event crossing `threshold` whose LOCAL date
/// is `date`, if it exists that date. `evening` selects the evening crossing
/// (sunset/dusk). Returns `None` on polar day/night dates.
fn solar_event_for_local_date(
    tz: &TimeZone,
    date: Date,
    latitude: f64,
    longitude: f64,
    threshold: f64,
    evening: bool,
) -> Option<i64> {
    // Local noon's UTC civil date is the centre of the search: the crossing
    // on local date `date` must fall on that UTC date or one day either side
    // (extreme longitudes/offsets). Exactly one candidate maps back to the
    // requested local date.
    let noon_utc = resolve_local(date.at(12, 0, 0, 0), tz)?;
    let (year, month, day) = utc_civil_date(noon_utc);
    let noon_days = noaa::days_from_civil(year, month, day);
    for shift in [0i64, -1, 1] {
        let (candidate_year, candidate_month, candidate_day) =
            noaa::civil_from_days(noon_days + shift);
        let crossing = if evening {
            noaa::solar_event_utc_ms_evening(
                candidate_year,
                candidate_month,
                candidate_day,
                latitude,
                longitude,
                threshold,
            )
        } else {
            noaa::solar_event_utc_ms(
                candidate_year,
                candidate_month,
                candidate_day,
                latitude,
                longitude,
                threshold,
            )
        };
        if let Some(utc_ms) = crossing
            && local_date_of_utc(tz, utc_ms) == Some(date)
        {
            return Some(utc_ms);
        }
    }
    None
}

/// Resolves a civil datetime in `tz` to UTC milliseconds. Nonexistent local
/// times (DST spring-forward gap) yield `None`; ambiguous local times (DST
/// fall-back fold) resolve to the EARLIER UTC instant.
fn resolve_local(dt: DateTime, tz: &TimeZone) -> Option<i64> {
    // jiff's compatible strategy: fold -> earlier offset, gap -> later offset.
    // Comparing the round-tripped civil datetime detects the gap (skipped).
    let zoned = dt.to_zoned(tz.clone()).ok()?;
    if zoned.datetime() != dt {
        return None;
    }
    Some(zoned.timestamp().as_millisecond())
}

fn local_datetime_of(tz: &TimeZone, utc_ms: i64) -> Option<DateTime> {
    let timestamp = Timestamp::from_millisecond(utc_ms).ok()?;
    Some(Zoned::new(timestamp, tz.clone()).datetime())
}

fn local_date_of_utc(tz: &TimeZone, utc_ms: i64) -> Option<Date> {
    local_datetime_of(tz, utc_ms).map(|dt| dt.date())
}

fn utc_civil_date(utc_ms: i64) -> (i32, u32, u32) {
    noaa::civil_from_days(utc_ms.div_euclid(86_400_000))
}

fn local_time_of(dt: DateTime) -> LocalTime {
    LocalTime {
        hour: dt.hour() as u8,
        minute: dt.minute() as u8,
        second: dt.second() as u8,
    }
}

fn weekday_of(date: Date) -> Weekday {
    match date.weekday() {
        JiffWeekday::Monday => Weekday::Monday,
        JiffWeekday::Tuesday => Weekday::Tuesday,
        JiffWeekday::Wednesday => Weekday::Wednesday,
        JiffWeekday::Thursday => Weekday::Thursday,
        JiffWeekday::Friday => Weekday::Friday,
        JiffWeekday::Saturday => Weekday::Saturday,
        JiffWeekday::Sunday => Weekday::Sunday,
    }
}

fn is_valid_local_time(at: &LocalTime) -> bool {
    at.hour <= 23 && at.minute <= 59 && at.second <= 59
}

/// The [`DateTimeValue`] of the solar event on `date`, or an unavailable
/// sentinel when no event exists that date (polar day/night).
fn event_value(
    tz: &TimeZone,
    date: Date,
    coordinates: Coordinates,
    threshold: f64,
    evening: bool,
) -> DateTimeValue {
    match solar_event_for_local_date(
        tz,
        date,
        coordinates.latitude,
        coordinates.longitude,
        threshold,
        evening,
    ) {
        Some(utc_ms) => datetime_value_at(tz, utc_ms),
        None => DateTimeValue::unavailable(),
    }
}

fn datetime_value_at(tz: &TimeZone, utc_ms: i64) -> DateTimeValue {
    match local_datetime_of(tz, utc_ms) {
        Some(dt) => DateTimeValue {
            available: true,
            year: Some(i32::from(dt.year())),
            month: Some(dt.month() as u8),
            day: Some(dt.day() as u8),
            hour: Some(dt.hour() as u8),
            minute: Some(dt.minute() as u8),
            second: Some(dt.second() as u8),
            weekday: Some(weekday_of(dt.date())),
            instant: Some(utc_ms),
        },
        None => DateTimeValue::unavailable(),
    }
}
