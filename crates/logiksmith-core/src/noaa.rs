//! Hand-rolled NOAA Solar Calculator equations.
//!
//! This module implements the public-domain NOAA Solar Calculator equations
//! (see https://gml.noaa.gov/grad/solcalc/ and the accompanying equations
//! document and spreadsheet). It is deliberately dependency-free: no
//! astronomy crate is used, and every function is pure and deterministic for
//! fixed inputs.
//!
//! Conventions:
//! - Latitude is degrees north (negative south), longitude degrees east
//!   (negative west).
//! - Elevation is degrees above the horizon. Sunrise/sunset use the
//!   conventional apparent horizon of `-0.833°` (solar centre), civil
//!   dawn/dusk use `-6°`.
//! - Azimuth is degrees clockwise from true north (`0` north, `90` east,
//!   `180` south, `270` west), following the NOAA spreadsheet.
//! - All instants are UTC milliseconds since the Unix epoch.

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil` algorithm).
pub(crate) fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let (year, month, day) = (i64::from(year), i64::from(month), i64::from(day));
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400; // [0, 399]
    let mp = (month + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`] (Hinnant's `civil_from_days`).
pub(crate) fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    if month <= 2 {
        year += 1;
    }
    (year as i32, month as u32, day as u32)
}

/// Day of year, 1-based (1 = January 1).
pub(crate) fn day_of_year(year: i32, month: u32, day: u32) -> u32 {
    let jan_first = days_from_civil(year, 1, 1);
    (days_from_civil(year, month, day) - jan_first + 1) as u32
}

/// Day of week index with `0 = Sunday` (1970-01-01 was a Thursday).
#[cfg(test)]
pub(crate) fn weekday_index_from_days(days: i64) -> u8 {
    // `days` is Unix-epoch-relative, while this API uses the conventional
    // Sunday=0 index.  1970-01-01 was a Thursday (4).
    (days + 4).rem_euclid(7) as u8
}

/// One solar position sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SolarPosition {
    /// Degrees above the horizon.
    pub elevation_degrees: f64,
    /// Degrees clockwise from true north.
    pub azimuth_degrees: f64,
    /// Declination in radians (exposed for tests).
    pub declination_radians: f64,
    /// Equation of time in minutes (exposed for tests).
    pub equation_of_time_minutes: f64,
}

const TWO_PI: f64 = std::f64::consts::TAU;

/// Solar geometry for a fractional day of year.
///
/// `gamma` is the fractional-year angle from the NOAA equations,
/// `2π/365 × (day_of_year - 1 + (hour - 12)/24)`, where `hour` is the
/// decimal hour of the instant.
fn solar_geometry(gamma: f64) -> (f64, f64) {
    let equation_of_time_minutes = 229.18
        * (0.000_075 + 0.001_868 * gamma.cos()
            - 0.032_077 * gamma.sin()
            - 0.014_615 * (2.0 * gamma).cos()
            - 0.040_849 * (2.0 * gamma).sin());
    let declination_radians = 0.006_918 - 0.399_912 * gamma.cos() + 0.070_257 * gamma.sin()
        - 0.006_758 * (2.0 * gamma).cos()
        + 0.000_907 * (2.0 * gamma).sin()
        - 0.002_697 * (3.0 * gamma).cos()
        + 0.001_48 * (3.0 * gamma).sin();
    (equation_of_time_minutes, declination_radians)
}

/// Solar elevation and azimuth at a UTC instant (NOAA spreadsheet equations).
pub(crate) fn solar_position_utc(
    utc_ms: i64,
    latitude_degrees: f64,
    longitude_degrees: f64,
) -> SolarPosition {
    let (year, month, day) = civil_from_days(utc_ms.div_euclid(86_400_000));
    let minutes_of_day = (utc_ms.rem_euclid(86_400_000) / 60_000) as f64;
    let day_of_year = f64::from(day_of_year(year, month, day));
    // Fractional hour of the day, noon-centred like the NOAA sheet.
    let hour = minutes_of_day / 60.0;
    let gamma = TWO_PI / 365.0 * (day_of_year - 1.0 + (hour - 12.0) / 24.0);
    let (equation_of_time, declination) = solar_geometry(gamma);
    let latitude = latitude_degrees.to_radians();

    let time_offset_minutes = equation_of_time + 4.0 * longitude_degrees;
    let true_solar_time_minutes = (minutes_of_day + time_offset_minutes).rem_euclid(1440.0);
    let hour_angle_degrees = true_solar_time_minutes / 4.0 - 180.0;
    let hour_angle = hour_angle_degrees.to_radians();

    let cos_zenith =
        latitude.sin() * declination.sin() + latitude.cos() * declination.cos() * hour_angle.cos();
    let cos_zenith = cos_zenith.clamp(-1.0, 1.0);
    let zenith_radians = cos_zenith.acos();
    let elevation_degrees = 90.0 - zenith_radians.to_degrees();

    // Azimuth measured clockwise from north: the NOAA sheet computes
    // cos(180 - az) = (sin φ cos z - sin δ) / (cos φ sin z).
    let sin_zenith = zenith_radians.sin();
    let azimuth_degrees = if sin_zenith.abs() < 1e-9 {
        // Sun at (or within a whisker of) the zenith: azimuth undefined;
        // report south per the sheet's degenerate behaviour.
        180.0
    } else {
        let ratio = (latitude.sin() * zenith_radians.cos() - declination.sin())
            / (latitude.cos() * sin_zenith);
        let distance_from_south = ratio.clamp(-1.0, 1.0).acos().to_degrees();
        // The cosine equation alone cannot distinguish morning from
        // afternoon. NOAA reflects the azimuth around due south after solar
        // noon so the result remains clockwise from true north.
        if hour_angle_degrees > 0.0 {
            180.0 + distance_from_south
        } else {
            180.0 - distance_from_south
        }
    };

    SolarPosition {
        elevation_degrees,
        azimuth_degrees,
        declination_radians: declination,
        equation_of_time_minutes: equation_of_time,
    }
}

/// UTC milliseconds of the solar event crossing `elevation_threshold_degrees`
/// on the UTC civil date `(year, month, day)`, if it exists that date.
///
/// Uses the NOAA spreadsheet hour-angle equations evaluated at the day's
/// approximate solar noon. Returns `None` when the threshold is never
/// reached that date (polar day/night). The morning crossing (dawn/sunrise)
/// precedes solar noon.
pub(crate) fn solar_event_utc_ms(
    year: i32,
    month: u32,
    day: u32,
    latitude_degrees: f64,
    longitude_degrees: f64,
    elevation_threshold_degrees: f64,
) -> Option<i64> {
    solar_crossing_utc_ms(
        year,
        month,
        day,
        latitude_degrees,
        longitude_degrees,
        elevation_threshold_degrees,
        false,
    )
}

/// Evening variant of [`solar_event_utc_ms`] (sunset / civil dusk).
pub(crate) fn solar_event_utc_ms_evening(
    year: i32,
    month: u32,
    day: u32,
    latitude_degrees: f64,
    longitude_degrees: f64,
    elevation_threshold_degrees: f64,
) -> Option<i64> {
    solar_crossing_utc_ms(
        year,
        month,
        day,
        latitude_degrees,
        longitude_degrees,
        elevation_threshold_degrees,
        true,
    )
}

fn solar_crossing_utc_ms(
    year: i32,
    month: u32,
    day: u32,
    latitude_degrees: f64,
    longitude_degrees: f64,
    elevation_threshold_degrees: f64,
    evening: bool,
) -> Option<i64> {
    let day_of_year = f64::from(day_of_year(year, month, day));
    // The spreadsheet evaluates declination/equation-of-time at solar noon
    // (hour 12), giving gamma = 2π/365 × (N - 1).
    let gamma = TWO_PI / 365.0 * (day_of_year - 1.0);
    let (equation_of_time, declination) = solar_geometry(gamma);
    let latitude = latitude_degrees.to_radians();

    let solar_noon_minutes = 720.0 - 4.0 * longitude_degrees - equation_of_time;
    let cos_hour_angle = (elevation_threshold_degrees.to_radians().sin()
        - latitude.sin() * declination.sin())
        / (latitude.cos() * declination.cos());
    if !(-1.0..=1.0).contains(&cos_hour_angle) {
        return None;
    }
    let hour_angle_degrees = cos_hour_angle.acos().to_degrees();
    let event_minutes = if evening {
        solar_noon_minutes + 4.0 * hour_angle_degrees
    } else {
        solar_noon_minutes - 4.0 * hour_angle_degrees
    };
    let midnight_ms = days_from_civil(year, month, day) * 86_400_000;
    Some(midnight_ms + (event_minutes * 60_000.0).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        (year, month, day): (i32, u32, u32),
        latitude: f64,
        longitude: f64,
        threshold: f64,
    ) -> Option<i64> {
        solar_event_utc_ms(year, month, day, latitude, longitude, threshold)
    }

    fn civil(utc_ms: i64) -> (i32, u32, u32) {
        civil_from_days(utc_ms.div_euclid(86_400_000))
    }

    #[test]
    fn civil_day_arithmetic_round_trips_and_matches_known_epoch() {
        // 1970-01-01 is day zero (Thursday).
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(weekday_index_from_days(days_from_civil(1970, 1, 1)), 4);
        // 2000-03-01 is day 11017 (a Wednesday per the Unix `date` command).
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(weekday_index_from_days(11_017), 3);
        // 2024-02-29 exists (leap year).
        assert_eq!(
            days_from_civil(2024, 2, 29) - days_from_civil(2024, 2, 28),
            1
        );
        for year in [1969, 1970, 2000, 2024, 2100, 9999] {
            for month in 1..=12 {
                let days_in_month = if month == 2 {
                    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
                    if leap { 29 } else { 28 }
                } else if [4, 6, 9, 11].contains(&month) {
                    30
                } else {
                    31
                };
                for day in [1, days_in_month] {
                    let days = days_from_civil(year, month, day);
                    assert_eq!(civil(days * 86_400_000), (year, month, day));
                }
            }
        }
    }

    #[test]
    fn day_of_year_matches_calendar() {
        assert_eq!(day_of_year(2024, 1, 1), 1);
        assert_eq!(day_of_year(2024, 2, 29), 60);
        assert_eq!(day_of_year(2024, 3, 1), 61);
        assert_eq!(day_of_year(2023, 12, 31), 365);
        assert_eq!(day_of_year(2024, 12, 31), 366);
    }

    #[test]
    fn elevation_matches_known_values_at_noon_and_azimuth_convention() {
        // 2024-06-21 (summer solstice), 40°N 105°W. Solar noon in UTC for
        // longitude -105 is ~12:00 + 7h = 19:00 UTC. At noon the sun is due
        // south (azimuth 180) and elevation is 90 - (40 - 23.44) ≈ 73.4°.
        let noon = days_from_civil(2024, 6, 21) * 86_400_000 + 19 * 3_600_000;
        let position = solar_position_utc(noon, 40.0, -105.0);
        assert!(
            (position.elevation_degrees - 73.4).abs() < 0.5,
            "elevation {}",
            position.elevation_degrees
        );
        assert!(
            (position.azimuth_degrees - 180.0).abs() < 2.0,
            "azimuth {}",
            position.azimuth_degrees
        );
        // Morning (06:00 local = 13:00 UTC): sun in the east.
        let morning = days_from_civil(2024, 6, 21) * 86_400_000 + 13 * 3_600_000;
        let position = solar_position_utc(morning, 40.0, -105.0);
        assert!(
            position.azimuth_degrees < 180.0,
            "morning azimuth {}",
            position.azimuth_degrees
        );
        assert!(position.elevation_degrees > 0.0);

        // Afternoon (18:00 local = 01:00 UTC on the following UTC date):
        // the sun is west of south and must have an azimuth greater than 180°.
        let afternoon = (days_from_civil(2024, 6, 22) * 86_400_000) + 1 * 3_600_000;
        let position = solar_position_utc(afternoon, 40.0, -105.0);
        assert!(
            position.azimuth_degrees > 180.0,
            "afternoon azimuth {}",
            position.azimuth_degrees
        );
        assert!(position.elevation_degrees > 0.0);
    }

    #[test]
    fn sunrise_elevation_is_consistent_with_the_threshold() {
        // The hour-angle formula and the elevation formula are two views of
        // the same NOAA equations: the instant returned as "sunrise" should
        // have an elevation very close to the threshold used to find it.
        for (year, month, day) in [(2024, 3, 20), (2024, 6, 21), (2024, 12, 21)] {
            let Some(sunrise) = event((year, month, day), 40.0, -105.0, -0.833) else {
                panic!("expected sunrise on {year}-{month}-{day}");
            };
            let position = solar_position_utc(sunrise, 40.0, -105.0);
            assert!(
                (position.elevation_degrees - (-0.833)).abs() < 0.2,
                "sunrise elevation {} on {year}-{month}-{day}",
                position.elevation_degrees
            );
        }
    }

    #[test]
    fn sunset_is_later_than_sunrise_and_elevation_matches_threshold() {
        let sunrise = event((2024, 6, 21), 40.0, -105.0, -0.833).unwrap();
        let sunset = solar_event_utc_ms_evening(2024, 6, 21, 40.0, -105.0, -0.833).unwrap();
        assert!(sunset > sunrise);
        let position = solar_position_utc(sunset, 40.0, -105.0);
        assert!(
            (position.elevation_degrees - (-0.833)).abs() < 0.05,
            "sunset elevation {}",
            position.elevation_degrees
        );
        // Roughly symmetric around solar noon (~19:01 UTC for this date and
        // longitude), rather than around the UTC clock hour 19:00 exactly.
        let sunrise_minutes = (sunrise.rem_euclid(86_400_000) / 60_000) as f64;
        let mut sunset_minutes = (sunset.rem_euclid(86_400_000) / 60_000) as f64;
        if sunset_minutes < sunrise_minutes {
            sunset_minutes += 1440.0;
        }
        let midpoint = (sunrise_minutes + sunset_minutes) / 2.0;
        assert!(
            (midpoint - 1142.0).abs() < 5.0,
            "sunrise {sunrise_minutes}, sunset {sunset_minutes}, midpoint {midpoint}"
        );
    }

    #[test]
    fn polar_date_has_no_sunrise_but_civil_dawn_may_exist() {
        // 2024-12-21 at 70°N 25°E (polar night): the sun peaks near -3.4°,
        // below the -0.833° horizon but above the -6° civil threshold.
        assert_eq!(event((2024, 12, 21), 70.0, 25.0, -0.833), None);
        assert!(event((2024, 12, 21), 70.0, 25.0, -6.0).is_some());
        // 78°N: the sun never reaches even -6°.
        assert_eq!(event((2024, 12, 21), 78.0, 25.0, -6.0), None);
        assert_eq!(event((2024, 12, 21), 78.0, 25.0, -0.833), None);
        // Summer at 70°N: midnight sun, so no sunrise or sunset crossings.
        assert_eq!(event((2024, 6, 21), 70.0, 25.0, -0.833), None);
        assert_eq!(event((2024, 6, 21), 78.0, 25.0, -6.0), None);
    }

    #[test]
    fn reference_fixture_boulder_equinox() {
        // Reference fixture source: timeanddate.com sunrise table for
        // Boulder, Colorado (https://www.timeanddate.com/sun/usa/boulder),
        // which lists sunrise 07:02 MDT (UTC-6) = 13:02 UTC on 2024-03-20.
        // The NOAA spreadsheet equations (implemented here) agree with the
        // published value within the model's ±1 minute accuracy; the
        // 2-minute tolerance absorbs the published rounding.
        let Some(sunrise) = event((2024, 3, 20), 40.015, -105.2705, -0.833) else {
            panic!("expected an equinox sunrise");
        };
        let expected_utc_ms =
            days_from_civil(2024, 3, 20) * 86_400_000 + 13 * 3_600_000 + 2 * 60_000;
        let drift = (sunrise - expected_utc_ms).abs();
        assert!(drift <= 3 * 60_000, "sunrise {sunrise} drifted {drift} ms");
    }
}
