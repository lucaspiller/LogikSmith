    #[test]
    fn local_config_example_contains_the_required_bridge_section() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/local.toml.example");
        let source = fs::read_to_string(path).expect("local config example should be present");
        let config: RawConfig = toml::from_str(&source).expect("local config example should parse");
        assert_eq!(config.bridge.python, ".venv/bin/python");
    }

    fn block(id: &str, input_address: &str, output_address: &str) -> AutomationBlock {
        AutomationBlock {
            id: id.to_owned(),
            revision: 1,
            enabled: true,
            inputs: vec![AutomationEndpoint {
                name: "input".to_owned(),
                dpt: "1.001".to_owned(),
            }],
            outputs: vec![AutomationEndpoint {
                name: "light".to_owned(),
                dpt: "1.001".to_owned(),
            }],
            knx_bindings: vec![
                KnxBinding {
                    endpoint: "input".to_owned(),
                    group_address: input_address.to_owned(),
                },
                KnxBinding {
                    endpoint: "light".to_owned(),
                    group_address: output_address.to_owned(),
                },
            ],
            source: "function handle(event, input) return nil end".to_owned(),
            schedules: Vec::new(),
        }
    }

    #[test]
    fn nested_document_supports_sixty_four_blocks_and_rejects_sixty_five() {
        let document = AutomationDocument {
            blocks: (0..64)
                .map(|index| {
                    block(
                        &format!("block_{index}"),
                        &format!("1/1/{}", index + 1),
                        &format!("1/2/{}", index + 1),
                    )
                })
                .collect(),
        };
        let runtime = build_automation(document.clone()).unwrap();
        assert_eq!(runtime.blocks.len(), 64);
        let too_many = AutomationDocument {
            blocks: (0..65)
                .map(|index| {
                    block(
                        &format!("block_{index}"),
                        &format!("2/1/{}", index + 1),
                        &format!("2/2/{}", index + 1),
                    )
                })
                .collect(),
        };
        assert!(build_automation(too_many).is_err());
    }

    #[test]
    fn structural_revision_ignores_source_enabled_and_persisted_block_revision() {
        let mut first = AutomationDocument {
            blocks: vec![block("one", "6/1/1", "6/2/1")],
        };
        let mut second = first.clone();
        second.blocks[0].source = "function handle(event) return nil end".to_owned();
        second.blocks[0].enabled = false;
        second.blocks[0].revision = 91;
        assert_eq!(structural_revision(&first), structural_revision(&second));

        first.blocks[0].inputs[0].dpt = "5.001".to_owned();
        assert_ne!(structural_revision(&first), structural_revision(&second));
    }

    #[test]
    fn schedule_duration_order_and_interval_weekdays_are_rejected() {
        assert_eq!(parse_duration_seconds("1h30m", false), Ok(5_400));
        assert_eq!(parse_duration_seconds("30m1h", false), Err(()));

        let schedule = AutomationSchedule {
            name: "heartbeat".to_owned(),
            enabled: true,
            kind: "interval".to_owned(),
            at: None,
            every: Some("60s".to_owned()),
            offset: None,
            anchor: None,
            earliest: None,
            latest: None,
            weekdays: Some(vec!["mon".to_owned()]),
            extra: Default::default(),
        };
        let mut errors = Vec::new();
        let _ = schedule_rule(&schedule, "blocks[0].schedules[0]", &mut errors);
        assert!(errors.iter().any(|error| error.path.ends_with(".weekdays")));
    }

    #[test]
    fn shared_same_dpt_address_fans_out_in_declaration_order() {
        let runtime = build_automation(AutomationDocument {
            blocks: vec![
                block("first", "3/1/1", "3/2/1"),
                block("second", "3/1/1", "3/2/2"),
            ],
        })
        .unwrap();
        let bindings = runtime
            .address_to_inputs
            .get(&GroupAddress::parse("3/1/1").unwrap())
            .unwrap();
        assert_eq!(
            bindings
                .iter()
                .map(|binding| binding.block_id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(runtime.address_dpts.len(), 3);
    }

    #[test]
    fn cross_block_dpt_conflict_and_local_duplicate_address_are_rejected() {
        let mut conflicting = block("second", "4/1/1", "4/2/1");
        conflicting.inputs[0].dpt = "5.001".to_owned();
        let errors = build_automation(AutomationDocument {
            blocks: vec![block("first", "4/1/1", "4/2/2"), conflicting],
        })
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.path == "blocks[1].knx_bindings[0].group_address")
        );

        let mut duplicate = block("first", "5/1/1", "5/2/1");
        duplicate.knx_bindings[1].group_address = "5/1/1".to_owned();
        let errors = build_automation(AutomationDocument {
            blocks: vec![duplicate],
        })
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.path == "blocks[0].knx_bindings[1].group_address")
        );
    }

    #[test]
    fn legacy_top_level_shape_reports_migration_fields() {
        let path =
            std::env::temp_dir().join(format!("logiksmith-legacy-{}.toml", std::process::id()));
        fs::write(&path, "[logic]\nsource = \"function handle() end\"\n").unwrap();
        let error = load_automation(&path).unwrap_err();
        assert!(
            matches!(error, AutomationFileError::Invalid(errors) if errors.iter().any(|error| error.path == "logic"))
        );
        let _ = fs::remove_file(path);
    }
