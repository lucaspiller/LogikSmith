    // --- helpers -------------------------------------------------------------

    fn utc_ms(year: i16, month: i8, day: i8, hour: i8, minute: i8, second: i8) -> i64 {
        Date::new(year, month, day)
            .unwrap()
            .at(hour, minute, second, 0)
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp()
            .as_millisecond()
    }
    fn sample(utc: i64) -> ClockSample {
        ClockSample {
            monotonic_ms: MonotonicMs(utc.max(0) as u64),
            utc_unix_ms: Some(utc),
        }
    }

    fn sample_at(monotonic_ms: u64, utc_unix_ms: Option<i64>) -> ClockSample {
        ClockSample {
            monotonic_ms: MonotonicMs(monotonic_ms),
            utc_unix_ms,
        }
    }

    fn id(value: &str) -> BlockId {
        value.parse().unwrap()
    }

    fn sname(value: &str) -> ScheduleName {
        ScheduleName::new(value).unwrap()
    }

    fn utc_site() -> SiteTimeConfig {
        SiteTimeConfig {
            timezone: TimeZoneId::utc(),
            coordinates: Some(Coordinates {
                latitude: 0.0,
                longitude: 0.0,
            }),
        }
    }

    fn nyc_site() -> SiteTimeConfig {
        SiteTimeConfig {
            timezone: TimeZoneId::new("America/New_York").unwrap(),
            coordinates: Some(Coordinates {
                latitude: 40.7,
                longitude: -74.0,
            }),
        }
    }

    fn fixed(at: LocalTime, weekdays: &[Weekday]) -> ScheduleRule {
        ScheduleRule::Fixed {
            at,
            weekdays: WeekdaySet::new(weekdays).unwrap(),
        }
    }

    fn interval(every_seconds: u32, offset_seconds: u32) -> ScheduleRule {
        ScheduleRule::Interval {
            every_seconds,
            offset_seconds,
        }
    }

    fn astro(
        anchor: SolarAnchor,
        offset_seconds: i32,
        weekdays: &[Weekday],
    ) -> ScheduleRule {
        ScheduleRule::Astronomical {
            anchor,
            offset_seconds,
            weekdays: WeekdaySet::new(weekdays).unwrap(),
        }
    }

    fn schedule(name: &str, enabled: bool, rule: ScheduleRule) -> BlockSchedule {
        BlockSchedule {
            name: sname(name),
            enabled,
            rule,
        }
    }

    fn block_config(id_value: &str, enabled: bool, schedules: Vec<BlockSchedule>) -> BlockConfig {
        BlockConfig::with_schedules(
            id(id_value),
            enabled,
            vec![
                Endpoint::input("input".parse().unwrap(), Dpt::BOOL),
                Endpoint::output("light".parse().unwrap(), Dpt::BOOL),
            ],
            "function handle(event) return {} end",
            schedules,
        )
    }

    fn event(value: bool) -> InputEvent {
        InputEvent::new("input".parse().unwrap(), TypedValue::bool(value))
    }

    fn trigger(
        block_id: &str,
        name: &str,
        scheduled_for_utc_ms: i64,
        structural_revision: u64,
    ) -> ScheduleTrigger {
        ScheduleTrigger {
            block_id: id(block_id),
            name: sname(name),
            kind: ScheduleKind::Interval,
            scheduled_for_utc_ms,
            detected_at_utc_ms: scheduled_for_utc_ms,
            coalesced_count: 0,
            structural_revision,
        }
    }

    fn weekday_from_index(index: u8) -> Weekday {
        match index {
            0 => Weekday::Sunday,
            1 => Weekday::Monday,
            2 => Weekday::Tuesday,
            3 => Weekday::Wednesday,
            4 => Weekday::Thursday,
            5 => Weekday::Friday,
            _ => Weekday::Saturday,
        }
    }

    /// Weekdays of every polar sunrise crossing inside the engine's 370-day
    /// search window from `baseline` (anchors baseline date - 1 .. + 369).
    fn polar_window_crossing_weekdays(baseline: i64) -> Vec<Weekday> {
        let start = baseline.div_euclid(86_400_000) - 1;
        let mut weekdays = Vec::new();
        for days in start..=start + SEARCH_DAY_LIMIT - 1 {
            let (year, month, day) = noaa::civil_from_days(days);
            if noaa::solar_event_utc_ms(year, month, day, 89.5, 0.0, SUNRISE_THRESHOLD_DEGREES)
                .is_some()
            {
                weekdays.push(weekday_from_index(noaa::weekday_index_from_days(days)));
            }
        }
        weekdays
    }

    // --- type-level tests ----------------------------------------------------

    #[test]
    fn weekday_names_and_all_in_calendar_order() {
        assert_eq!(Weekday::ALL.len(), 7);
        assert_eq!(
            Weekday::ALL,
            [
                Weekday::Monday,
                Weekday::Tuesday,
                Weekday::Wednesday,
                Weekday::Thursday,
                Weekday::Friday,
                Weekday::Saturday,
                Weekday::Sunday,
            ]
        );
        assert_eq!(Weekday::Monday.to_string(), "Monday");
        assert_eq!(Weekday::Sunday.to_string(), "Sunday");
        assert_eq!(format!("{}", Weekday::Wednesday), "Wednesday");
    }

    #[test]
    fn weekday_set_validates_nonempty_unique_and_displays() {
        assert_eq!(WeekdaySet::new(&[]), Err(WeekdaySetError::Empty));
        assert_eq!(
            WeekdaySet::new(&[Weekday::Monday, Weekday::Monday]),
            Err(WeekdaySetError::Duplicate(Weekday::Monday))
        );
        let set = WeekdaySet::new(&[Weekday::Friday, Weekday::Monday]).unwrap();
        assert_eq!(set.to_string(), "Monday, Friday");
        assert!(set.contains(Weekday::Monday));
        assert!(!set.contains(Weekday::Sunday));
    }

    #[test]
    fn schedule_name_uses_endpoint_grammar() {
        assert!(ScheduleName::new("off").is_ok());
        assert!(ScheduleName::new("a.b_c-2").is_ok());
        assert!(ScheduleName::new("").is_err());
        assert!(ScheduleName::new("9x").is_err());
        assert!(ScheduleName::new("a b").is_err());
        assert_eq!(sname("off").as_str(), "off");
        assert_eq!(sname("off").to_string(), "off");
    }

    #[test]
    fn timezone_id_validates_iana_names_and_utc_shortcut() {
        assert!(TimeZoneId::new("Europe/Berlin").is_ok());
        assert!(TimeZoneId::new("America/New_York").is_ok());
        assert!(TimeZoneId::new("Mars/Olympus").is_err());
        assert!(TimeZoneId::new("not a zone").is_err());
        assert_eq!(TimeZoneId::utc().as_str(), "UTC");
        assert_eq!(TimeZoneId::utc().to_string(), "UTC");
        assert_eq!(TimeZoneId::utc(), TimeZoneId::new("UTC").unwrap());
    }

    // --- fixed rules ---------------------------------------------------------

    #[test]
    fn fixed_occurrence_crosses_month_and_year_boundary() {
        let rule = fixed(
            LocalTime {
                hour: 7,
                minute: 30,
                second: 0,
            },
            &Weekday::ALL,
        );
        // 2026-12-31T20:00Z -> next day 07:30Z (crosses a year boundary).
        assert_eq!(
            next_occurrence_after(&rule, &utc_site(), utc_ms(2026, 12, 31, 20, 0, 0)),
            Some(utc_ms(2027, 1, 1, 7, 30, 0))
        );
        // Same instant baseline is strictly excluded -> next day.
        assert_eq!(
            next_occurrence_after(&rule, &utc_site(), utc_ms(2026, 6, 1, 7, 30, 0)),
            Some(utc_ms(2026, 6, 2, 7, 30, 0))
        );
        // Month boundary: late June 30 -> July 1.
        assert_eq!(
            next_occurrence_after(&rule, &utc_site(), utc_ms(2026, 6, 30, 23, 0, 0)),
            Some(utc_ms(2026, 7, 1, 7, 30, 0))
        );
    }

    #[test]
    fn fixed_occurrence_honours_weekday_filter() {
        let mondays = fixed(
            LocalTime {
                hour: 7,
                minute: 30,
                second: 0,
            },
            &[Weekday::Monday],
        );
        // 2026-12-31 is a Thursday -> next Monday is 2027-01-04.
        assert_eq!(
            next_occurrence_after(&mondays, &utc_site(), utc_ms(2026, 12, 31, 20, 0, 0)),
            Some(utc_ms(2027, 1, 4, 7, 30, 0))
        );
    }

    #[test]
    fn fixed_skips_nonexistent_dst_local_time() {
        // America/New_York spring-forward 2026-03-08 02:00 -> 03:00; 02:30
        // never exists. The next occurrence is 2026-03-09 02:30 EDT = 06:30Z.
        let rule = fixed(
            LocalTime {
                hour: 2,
                minute: 30,
                second: 0,
            },
            &Weekday::ALL,
        );
        assert_eq!(
            next_occurrence_after(&rule, &nyc_site(), utc_ms(2026, 3, 7, 12, 0, 0)),
            Some(utc_ms(2026, 3, 9, 6, 30, 0))
        );
    }

    #[test]
    fn fixed_ambiguous_fires_once_at_earlier_utc() {
        // America/New_York fall-back 2026-11-01: 01:30 occurs twice (EDT then
        // EST); the earlier UTC instant is 01:30 EDT = 05:30Z.
        let rule = fixed(
            LocalTime {
                hour: 1,
                minute: 30,
                second: 0,
            },
            &Weekday::ALL,
        );
        assert_eq!(
            next_occurrence_after(&rule, &nyc_site(), utc_ms(2026, 10, 31, 12, 0, 0)),
            Some(utc_ms(2026, 11, 1, 5, 30, 0))
        );
    }

    // --- interval rules ------------------------------------------------------

    #[test]
    fn interval_phase_survives_restart_and_source_edits() {
        let hourly = interval(3600, 0);
        // Phase is anchored to the Unix epoch, not to the baseline: restarting
        // at a later instant keeps the same absolute grid.
        assert_eq!(
            next_occurrence_after(&hourly, &utc_site(), utc_ms(2026, 6, 1, 10, 15, 0)),
            Some(utc_ms(2026, 6, 1, 11, 0, 0))
        );
        assert_eq!(
            next_occurrence_after(&hourly, &utc_site(), utc_ms(2026, 6, 1, 10, 45, 0)),
            Some(utc_ms(2026, 6, 1, 11, 0, 0))
        );
        // Editing the offset shifts the phase to the new rule.
        let shifted = interval(3600, 1800);
        assert_eq!(
            next_occurrence_after(&shifted, &utc_site(), utc_ms(2026, 6, 1, 10, 45, 0)),
            Some(utc_ms(2026, 6, 1, 11, 30, 0))
        );
    }

    #[test]
    fn interval_is_strictly_after_baseline() {
        let hourly = interval(3600, 0);
        assert_eq!(
            next_occurrence_after(&hourly, &utc_site(), utc_ms(2026, 6, 1, 11, 0, 0)),
            Some(utc_ms(2026, 6, 1, 12, 0, 0))
        );
        let offset = interval(3600, 600);
        assert_eq!(
            next_occurrence_after(&offset, &utc_site(), utc_ms(2026, 6, 1, 11, 10, 0)),
            Some(utc_ms(2026, 6, 1, 12, 10, 0))
        );
    }

    // --- astronomical rules --------------------------------------------------

    #[test]
    fn astronomical_offset() {
        let site = utc_site();
        let baseline = utc_ms(2026, 6, 1, 0, 0, 0);
        // Plain offset: sunrise at (0,0) + 1h, computed from the NOAA engine.
        let plain = astro(SolarAnchor::Sunrise, 3600, &Weekday::ALL);
        let sunrise_june_1 =
            noaa::solar_event_utc_ms(2026, 6, 1, 0.0, 0.0, SUNRISE_THRESHOLD_DEGREES)
                .expect("equator sunrise exists");
        assert_eq!(
            next_occurrence_after(&plain, &site, baseline),
            Some(sunrise_june_1 + 3_600_000)
        );
    }

    #[test]
    fn astronomical_weekday_filter_on_final_date() {
        // 2026-06-01 is a Monday; a Saturday-only rule fires on June 6.
        let saturdays = astro(SolarAnchor::Sunrise, 3600, &[Weekday::Saturday]);
        let sunrise_june_6 =
            noaa::solar_event_utc_ms(2026, 6, 6, 0.0, 0.0, SUNRISE_THRESHOLD_DEGREES)
                .expect("equator sunrise exists");
        assert_eq!(
            next_occurrence_after(&saturdays, &utc_site(), utc_ms(2026, 6, 1, 0, 0, 0)),
            Some(sunrise_june_6 + 3_600_000)
        );
    }

    #[test]
    fn astronomical_polar_unavailable_after_370_days() {
        let polar_site = SiteTimeConfig {
            timezone: TimeZoneId::utc(),
            coordinates: Some(Coordinates {
                latitude: 89.5,
                longitude: 0.0,
            }),
        };
        let baseline = utc_ms(2026, 7, 1, 0, 0, 0);
        // Near the pole the morning crossing occurs twice a year (the October
        // descent and the March ascent); the engine still reaches the next
        // one inside the 370-day window.
        let mut next_crossing = None;
        let mut crossing_weekdays = Vec::new();
        let start = baseline.div_euclid(86_400_000) - 1;
        for days in start..=start + SEARCH_DAY_LIMIT - 1 {
            let (year, month, day) = noaa::civil_from_days(days);
            if let Some(ms) =
                noaa::solar_event_utc_ms(year, month, day, 89.5, 0.0, SUNRISE_THRESHOLD_DEGREES)
            {
                crossing_weekdays.push(weekday_from_index(noaa::weekday_index_from_days(days)));
                if next_crossing.is_none() && ms > baseline {
                    next_crossing = Some(ms);
                }
            }
        }
        let next_crossing = next_crossing.expect("polar sunrise crossing within the window");
        let all = astro(SolarAnchor::Sunrise, 0, &Weekday::ALL);
        assert_eq!(
            next_occurrence_after(&all, &polar_site, baseline),
            Some(next_crossing)
        );

        // Exclude every weekday any in-window crossing falls on: no occurrence
        // remains inside the 370-day window -> unavailable.
        assert!(!crossing_weekdays.is_empty());
        let allowed: Vec<Weekday> = Weekday::ALL
            .iter()
            .copied()
            .filter(|weekday| !crossing_weekdays.contains(weekday))
            .collect();
        assert!(!allowed.is_empty());
        let filtered = astro(SolarAnchor::Sunrise, 0, &allowed);
        assert_eq!(
            next_occurrence_after(&filtered, &polar_site, baseline),
            None
        );
    }

    #[test]
    fn astronomical_without_coordinates_is_unavailable() {
        let no_coords = SiteTimeConfig {
            timezone: TimeZoneId::utc(),
            coordinates: None,
        };
        let rule = astro(SolarAnchor::Sunrise, 0, &Weekday::ALL);
        assert_eq!(
            next_occurrence_after(&rule, &no_coords, utc_ms(2026, 6, 1, 0, 0, 0)),
            None
        );
    }

    // --- polling, baseline, coalescing --------------------------------------

    #[test]
    fn future_only_baseline_and_statuses() {
        let schedules = vec![schedule(
            "daily",
            true,
            fixed(
                LocalTime {
                    hour: 10,
                    minute: 0,
                    second: 0,
                },
                &Weekday::ALL,
            ),
        )];
        let mut runtime = Runtime::new(RuntimeConfig::with_site(
            vec![block_config("a", true, schedules)],
            utc_site(),
        ));
        let baseline = utc_ms(2026, 6, 1, 10, 30, 0);
        runtime.initialise_schedules(sample(baseline), 1).unwrap();
        // Today's 10:00 already passed; the next occurrence is tomorrow.
        let statuses = runtime.schedule_statuses(Some(baseline));
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].status, ScheduleStatusKind::Active);
        assert_eq!(
            statuses[0].next_occurrence_utc_ms,
            Some(utc_ms(2026, 6, 2, 10, 0, 0))
        );
        // Re-initialising later (restart/re-enable) keeps the future-only rule.
        runtime
            .initialise_schedules(sample(utc_ms(2026, 6, 1, 23, 0, 0)), 2)
            .unwrap();
        let statuses = runtime.schedule_statuses(Some(utc_ms(2026, 6, 1, 23, 0, 0)));
        assert_eq!(
            statuses[0].next_occurrence_utc_ms,
            Some(utc_ms(2026, 6, 2, 10, 0, 0))
        );
    }

    #[test]
    fn poll_coalesces_passed_occurrences_with_count() {
        let mut runtime = Runtime::new(RuntimeConfig::with_site(
            vec![block_config(
                "a",
                true,
                vec![schedule("hourly", true, interval(3600, 0))],
            )],
            utc_site(),
        ));
        runtime
            .initialise_schedules(sample(utc_ms(2026, 6, 1, 10, 0, 0)), 1)
            .unwrap();
        // Poll at 13:30: occurrences at 11:00, 12:00, 13:00 all passed.
        let triggers = runtime
            .poll_schedules(sample(utc_ms(2026, 6, 1, 13, 30, 0)))
            .unwrap();
        assert_eq!(triggers.len(), 1);
        assert_eq!(
            triggers[0].scheduled_for_utc_ms,
            utc_ms(2026, 6, 1, 13, 0, 0)
        );
        assert_eq!(
            triggers[0].detected_at_utc_ms,
            utc_ms(2026, 6, 1, 13, 30, 0)
        );
        assert_eq!(triggers[0].coalesced_count, 2);
        assert_eq!(triggers[0].structural_revision, 1);
        assert_eq!(triggers[0].kind, ScheduleKind::Interval);
        // Cursor advanced past the poll instant; nothing due at 13:45.
        assert_eq!(
            runtime
                .poll_schedules(sample(utc_ms(2026, 6, 1, 13, 45, 0)))
                .unwrap(),
            vec![]
        );
        // The 14:00 occurrence fires without coalescing.
        let triggers = runtime
            .poll_schedules(sample(utc_ms(2026, 6, 1, 14, 0, 0)))
            .unwrap();
        assert_eq!(triggers.len(), 1);
        assert_eq!(
            triggers[0].scheduled_for_utc_ms,
            utc_ms(2026, 6, 1, 14, 0, 0)
        );
        assert_eq!(triggers[0].coalesced_count, 0);
    }

    #[test]
    fn poll_returns_deterministic_order() {
        let mut runtime = Runtime::new(RuntimeConfig::with_site(
            vec![
                block_config(
                    "a",
                    true,
                    vec![
                        schedule("a", true, interval(3600, 0)),
                        schedule("z", true, interval(3600, 0)),
                    ],
                ),
                block_config(
                    "b",
                    true,
                    vec![
                        schedule("a", true, interval(3600, 600)),
                        schedule("z", true, interval(3600, 1200)),
                    ],
                ),
            ],
            utc_site(),
        ));
        runtime
            .initialise_schedules(sample(utc_ms(2026, 6, 1, 10, 0, 0)), 1)
            .unwrap();
        let triggers = runtime
            .poll_schedules(sample(utc_ms(2026, 6, 1, 11, 30, 0)))
            .unwrap();
        let order: Vec<(i64, &str, &str)> = triggers
            .iter()
            .map(|t| (t.scheduled_for_utc_ms, t.block_id.as_str(), t.name.as_str()))
            .collect();
        assert_eq!(
            order,
            vec![
                (utc_ms(2026, 6, 1, 11, 0, 0), "a", "a"),
                (utc_ms(2026, 6, 1, 11, 0, 0), "a", "z"),
                (utc_ms(2026, 6, 1, 11, 10, 0), "b", "a"),
                (utc_ms(2026, 6, 1, 11, 20, 0), "b", "z"),
            ]
        );
    }

    #[test]
    fn invalid_clock_pauses_schedules_but_input_still_runs() {
        let mut runtime = Runtime::new(RuntimeConfig::with_site(
            vec![block_config(
                "a",
                true,
                vec![schedule("hourly", true, interval(3600, 0))],
            )],
            utc_site(),
        ));
        runtime
            .initialise_schedules(sample(utc_ms(2026, 6, 1, 10, 0, 0)), 1)
            .unwrap();
        let no_clock = ClockSample {
            monotonic_ms: MonotonicMs(utc_ms(2026, 6, 1, 10, 0, 1) as u64),
            utc_unix_ms: None,
        };
        // Poll with no wall clock -> paused, no triggers.
        assert_eq!(runtime.poll_schedules(no_clock).unwrap(), vec![]);
        assert_eq!(runtime.next_schedule_deadline(), None);
        assert_eq!(
            runtime.schedule_statuses(None)[0].status,
            ScheduleStatusKind::ClockError
        );
        // Inputs still run, with an unavailable time context.
        let execution = runtime
            .process_input_sampled(&id("a"), event(true), no_clock)
            .unwrap()
            .unwrap();
        assert!(!execution.execution.time_context.now.available);
        // With a wall clock the same input captures a real context.
        let execution = runtime
            .process_input_sampled(&id("a"), event(true), sample(utc_ms(2026, 6, 1, 10, 31, 0)))
            .unwrap()
            .unwrap();
        assert!(execution.execution.time_context.now.available);
        assert_eq!(execution.execution.time_context.now.year, Some(2026));
        assert_eq!(execution.execution.time_context.now.hour, Some(10));
    }

    #[test]
    fn first_valid_clock_sample_after_invalid_start_establishes_future_baseline() {
        let mut runtime = Runtime::new(RuntimeConfig::with_site(
            vec![block_config(
                "a",
                true,
                vec![schedule("hourly", true, interval(3600, 0))],
            )],
            utc_site(),
        ));
        assert_eq!(
            runtime.initialise_schedules(sample_at(10, None), 4),
            Err(TimeError::ClockUnavailable)
        );
        assert_eq!(runtime.next_schedule_deadline(), None);

        let baseline = utc_ms(2026, 6, 1, 10, 0, 0);
        assert_eq!(
            runtime
                .poll_schedules(sample_at(20, Some(baseline)))
                .unwrap(),
            vec![]
        );
        assert_eq!(
            runtime.next_schedule_deadline(),
            Some(UtcUnixMs(baseline + 3_600_000))
        );
        let triggers = runtime
            .poll_schedules(sample_at(30, Some(baseline + 3_600_000)))
            .unwrap();
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].scheduled_for_utc_ms, baseline + 3_600_000);
    }

    #[test]
    fn backward_wall_clock_recomputes_without_replaying_last_delivery() {
        let mut runtime = Runtime::new(RuntimeConfig::with_site(
            vec![block_config(
                "a",
                true,
                vec![schedule("hourly", true, interval(3600, 0))],
            )],
            utc_site(),
        ));
        let ten = utc_ms(2026, 6, 1, 10, 0, 0);
        runtime
            .initialise_schedules(sample_at(0, Some(ten)), 1)
            .unwrap();
        let noon = utc_ms(2026, 6, 1, 12, 0, 0);
        let first = runtime.poll_schedules(sample_at(10, Some(noon))).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].scheduled_for_utc_ms, noon);

        // A correction to 10:30 would ordinarily make 11:00 due. The last
        // delivered occurrence was 12:00, so the corrected cursor skips to
        // 13:00 instead of replaying 11:00.
        let corrected = utc_ms(2026, 6, 1, 10, 30, 0);
        assert_eq!(
            runtime
                .poll_schedules(sample_at(20, Some(corrected)))
                .unwrap(),
            vec![]
        );
        assert_eq!(
            runtime.next_schedule_deadline(),
            Some(UtcUnixMs(noon + 3_600_000))
        );
        let next = runtime
            .poll_schedules(sample_at(30, Some(noon + 3_600_000)))
            .unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].scheduled_for_utc_ms, noon + 3_600_000);
    }

    #[test]
    fn reenable_rebaseline_ignores_occurrences_that_passed_while_disabled() {
        let mut runtime = Runtime::new(RuntimeConfig::with_site(
            vec![block_config(
                "a",
                true,
                vec![schedule("hourly", true, interval(3600, 0))],
            )],
            utc_site(),
        ));
        let ten = utc_ms(2026, 6, 1, 10, 0, 0);
        runtime
            .initialise_schedules(sample_at(0, Some(ten)), 1)
            .unwrap();
        runtime
            .activate(RuntimeActivation::single(BlockActivation::enabled(
                id("a"),
                false,
            )))
            .unwrap();
        runtime
            .activate(RuntimeActivation::single(BlockActivation::enabled(
                id("a"),
                true,
            )))
            .unwrap();
        let twelve = utc_ms(2026, 6, 1, 12, 0, 0);
        runtime
            .rebaseline_block_schedules(&id("a"), sample_at(10, Some(twelve)))
            .unwrap();
        assert_eq!(
            runtime.poll_schedules(sample_at(11, Some(twelve))).unwrap(),
            vec![]
        );
        let thirteen = utc_ms(2026, 6, 1, 13, 0, 0);
        let triggers = runtime
            .poll_schedules(sample_at(12, Some(thirteen)))
            .unwrap();
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].scheduled_for_utc_ms, thirteen);
    }

    #[test]
    fn schedule_statuses_reflect_engine_state() {
        let mut runtime = Runtime::new(RuntimeConfig::with_site(
            vec![block_config(
                "a",
                true,
                vec![
                    schedule("on", true, interval(3600, 0)),
                    schedule("off", false, interval(3600, 0)),
                ],
            )],
            utc_site(),
        ));
        runtime
            .initialise_schedules(sample(utc_ms(2026, 6, 1, 10, 0, 0)), 1)
            .unwrap();
        let statuses = runtime.schedule_statuses(Some(utc_ms(2026, 6, 1, 10, 0, 0)));
        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].block_id, id("a"));
        assert_eq!(statuses[0].name, sname("on"));
        assert!(statuses[0].enabled);
        assert_eq!(statuses[0].status, ScheduleStatusKind::Active);
        assert_eq!(
            statuses[0].next_occurrence_utc_ms,
            Some(utc_ms(2026, 6, 1, 11, 0, 0))
        );
        assert_eq!(statuses[1].name, sname("off"));
        assert!(!statuses[1].enabled);
        assert_eq!(statuses[1].status, ScheduleStatusKind::Paused);
        assert_eq!(statuses[1].next_occurrence_utc_ms, None);
        // No clock -> enabled schedules report ClockError.
        let statuses = runtime.schedule_statuses(None);
        assert_eq!(statuses[0].status, ScheduleStatusKind::ClockError);
        assert_eq!(statuses[1].status, ScheduleStatusKind::Paused);
        // Polar-unavailable schedule reports Unavailable with a reason.
        let polar_baseline = utc_ms(2026, 7, 1, 0, 0, 0);
        let polar_weekdays: Vec<Weekday> = Weekday::ALL
            .iter()
            .copied()
            .filter(|weekday| !polar_window_crossing_weekdays(polar_baseline).contains(weekday))
            .collect();
        let polar_site = SiteTimeConfig {
            timezone: TimeZoneId::utc(),
            coordinates: Some(Coordinates {
                latitude: 89.5,
                longitude: 0.0,
            }),
        };
        let mut polar_runtime = Runtime::new(RuntimeConfig::with_site(
            vec![block_config(
                "a",
                true,
                vec![schedule(
                    "polar",
                    true,
                    astro(SolarAnchor::Sunrise, 0, &polar_weekdays),
                )],
            )],
            polar_site,
        ));
        polar_runtime
            .initialise_schedules(sample(polar_baseline), 1)
            .unwrap();
        let statuses = polar_runtime.schedule_statuses(Some(polar_baseline));
        assert!(matches!(
            &statuses[0].status,
            ScheduleStatusKind::Unavailable { reason } if !reason.is_empty()
        ));
        assert_eq!(statuses[0].next_occurrence_utc_ms, None);
    }

    #[test]
    fn preview_occurrences_is_stateless_and_deterministic() {
        let mut runtime = Runtime::new(RuntimeConfig::with_site(
            vec![block_config(
                "a",
                true,
                vec![schedule("m", true, interval(3600, 600))],
            )],
            utc_site(),
        ));
        runtime
            .initialise_schedules(sample(utc_ms(2026, 6, 1, 10, 0, 0)), 1)
            .unwrap();
        let preview = runtime
            .preview_occurrences(&id("a"), &sname("m"), utc_ms(2026, 6, 1, 10, 0, 0), 3)
            .unwrap();
        assert_eq!(
            preview,
            vec![
                ScheduleOccurrence {
                    utc_ms: utc_ms(2026, 6, 1, 10, 10, 0)
                },
                ScheduleOccurrence {
                    utc_ms: utc_ms(2026, 6, 1, 11, 10, 0)
                },
                ScheduleOccurrence {
                    utc_ms: utc_ms(2026, 6, 1, 12, 10, 0)
                },
            ]
        );
        // Unknown block or schedule -> UnknownSchedule.
        assert_eq!(
            runtime.preview_occurrences(&id("nope"), &sname("m"), 0, 1),
            Err(ScheduleError::UnknownSchedule)
        );
        assert_eq!(
            runtime.preview_occurrences(&id("a"), &sname("nope"), 0, 1),
            Err(ScheduleError::UnknownSchedule)
        );
        // count == 0 -> empty, no error.
        assert_eq!(
            runtime
                .preview_occurrences(&id("a"), &sname("m"), 0, 0)
                .unwrap(),
            vec![]
        );
    }

    // --- process_schedule ----------------------------------------------------

    #[test]
    fn process_schedule_routes_to_lua_and_drops_stale() {
        let source = "function handle(event) if event.type == 'schedule' then return { state = { kind = event.kind, at = event.scheduled_for_utc_ms } } end return {} end";
        let mut runtime = Runtime::new(RuntimeConfig::with_site(
            vec![BlockConfig::with_schedules(
                id("a"),
                true,
                vec![
                    Endpoint::input("input".parse().unwrap(), Dpt::BOOL),
                    Endpoint::output("light".parse().unwrap(), Dpt::BOOL),
                ],
                source,
                vec![schedule("m", true, interval(3600, 0))],
            )],
            utc_site(),
        ));
        runtime
            .initialise_schedules(sample(utc_ms(2026, 6, 1, 10, 0, 0)), 7)
            .unwrap();
        let triggers = runtime
            .poll_schedules(sample(utc_ms(2026, 6, 1, 11, 0, 5)))
            .unwrap();
        assert_eq!(triggers.len(), 1);
        let due_trigger = triggers[0].clone();
        assert_eq!(due_trigger.structural_revision, 7);
        let processed = runtime
            .process_schedule(due_trigger.clone())
            .unwrap()
            .expect("enabled schedule processes");
        assert!(matches!(processed.execution.trigger, Trigger::Schedule(_)));
        assert_eq!(
            processed.execution.state_after["kind"],
            StateValue::String("interval".to_owned())
        );
        assert_eq!(
            processed.execution.state_after["at"],
            StateValue::Integer(utc_ms(2026, 6, 1, 11, 0, 0))
        );
        // Re-initialised with a new revision -> the old trigger is stale.
        runtime
            .initialise_schedules(sample(utc_ms(2026, 6, 1, 11, 1, 0)), 8)
            .unwrap();
        assert!(runtime.process_schedule(due_trigger).unwrap().is_none());
        // Unknown block, unknown schedule, disabled schedule, disabled block.
        let mut probe = Runtime::new(RuntimeConfig::with_site(
            vec![
                block_config("live", true, vec![schedule("on", true, interval(3600, 0))]),
                block_config(
                    "off_block",
                    false,
                    vec![schedule("on", true, interval(3600, 0))],
                ),
            ],
            utc_site(),
        ));
        probe
            .initialise_schedules(sample(utc_ms(2026, 6, 1, 10, 0, 0)), 1)
            .unwrap();
        assert!(
            probe
                .process_schedule(trigger("nope", "on", 0, 1))
                .unwrap()
                .is_none()
        );
        assert!(
            probe
                .process_schedule(trigger("live", "nope", 0, 1))
                .unwrap()
                .is_none()
        );
        assert!(
            probe
                .process_schedule(trigger("off_block", "on", 0, 1))
                .unwrap()
                .is_none()
        );
        let mut disabled_schedule = Runtime::new(RuntimeConfig::with_site(
            vec![block_config(
                "a",
                true,
                vec![schedule("off", false, interval(3600, 0))],
            )],
            utc_site(),
        ));
        disabled_schedule
            .initialise_schedules(sample(utc_ms(2026, 6, 1, 10, 0, 0)), 1)
            .unwrap();
        assert!(
            disabled_schedule
                .process_schedule(trigger("a", "off", 0, 1))
                .unwrap()
                .is_none()
        );
    }
