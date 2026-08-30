    use super::*;
    use crate::diagnostics::JOURNAL_CAPACITY;
    use logiksmith_core::Runtime;
    use std::{fs, net::IpAddr};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn store() -> DiagnosticStore {
        let runtime = crate::build_automation(crate::AutomationDocument {
            signals: Vec::new(),
            blocks: vec![crate::AutomationBlock {
                id: "test".to_owned(),
                revision: 1,
                enabled: true,
                inputs: vec![
                    crate::AutomationEndpoint {
                        name: "wall_switch".to_owned(),
                        dpt: "1.001".to_owned(),
                    },
                    crate::AutomationEndpoint {
                        name: "dimmer_level".to_owned(),
                        dpt: "5.001".to_owned(),
                    },
                ],
                outputs: vec![
                    crate::AutomationEndpoint {
                        name: "test_light".to_owned(),
                        dpt: "1.001".to_owned(),
                    },
                    crate::AutomationEndpoint {
                        name: "dimmer_output".to_owned(),
                        dpt: "5.001".to_owned(),
                    },
                ],
                knx_bindings: vec![
                    crate::KnxBinding {
                        endpoint: "wall_switch".to_owned(),
                        group_address: "2/2/52".to_owned(),
                    },
                    crate::KnxBinding {
                        endpoint: "dimmer_level".to_owned(),
                        group_address: "2/2/53".to_owned(),
                    },
                    crate::KnxBinding {
                        endpoint: "test_light".to_owned(),
                        group_address: "2/3/52".to_owned(),
                    },
                    crate::KnxBinding {
                        endpoint: "dimmer_output".to_owned(),
                        group_address: "2/3/53".to_owned(),
                    },
                ],
                signal_bindings: Vec::new(),
                source: "function handle(event, input) return nil end".to_owned(),
                schedules: Vec::new(),
            }],
        })
        .unwrap();
        DiagnosticStore::new(
            &runtime,
            std::env::temp_dir().join("logiksmith-web-test-automation.toml"),
            1,
        )
    }

    #[test]
    fn saved_document_revision_is_persisted_and_incremented() {
        let path = std::env::temp_dir().join(format!(
            "logiksmith-automation-revision-{}-{}.toml",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let document = AutomationDocument {
            signals: Vec::new(),
            blocks: vec![crate::AutomationBlock {
                id: "test".to_owned(),
                revision: 1,
                enabled: true,
                inputs: vec![crate::AutomationEndpoint {
                    name: "switch".to_owned(),
                    dpt: "1.001".to_owned(),
                }],
                outputs: vec![crate::AutomationEndpoint {
                    name: "light".to_owned(),
                    dpt: "1.001".to_owned(),
                }],
                knx_bindings: vec![
                    crate::KnxBinding {
                        endpoint: "switch".to_owned(),
                        group_address: "1/1/1".to_owned(),
                    },
                    crate::KnxBinding {
                        endpoint: "light".to_owned(),
                        group_address: "1/1/2".to_owned(),
                    },
                ],
                signal_bindings: Vec::new(),
                source: "function handle() end".to_owned(),
                schedules: Vec::new(),
            }],
        };
        fs::write(&path, serialize_automation(&document, 41).unwrap()).unwrap();

        assert_eq!(load_automation(&path).unwrap().1, 0);
        assert_eq!(atomic_save(&path, &document).unwrap(), 0);
        assert_eq!(load_automation(&path).unwrap().1, 0);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn block_revisions_increment_only_for_changed_blocks() {
        let current = AutomationDocument {
            signals: Vec::new(),
            blocks: vec![
                crate::AutomationBlock {
                    id: "first".to_owned(),
                    revision: 4,
                    enabled: true,
                    inputs: vec![],
                    outputs: vec![],
                    knx_bindings: vec![],
                    signal_bindings: vec![],
                    source: "return 1".to_owned(),
                    schedules: Vec::new(),
                },
                crate::AutomationBlock {
                    id: "second".to_owned(),
                    revision: 9,
                    enabled: true,
                    inputs: vec![],
                    outputs: vec![],
                    knx_bindings: vec![],
                    signal_bindings: vec![],
                    source: "return 2".to_owned(),
                    schedules: Vec::new(),
                },
            ],
        };
        let mut candidate = current.clone();
        candidate.blocks[0].source = "return 3".to_owned();
        merge_block_revisions(&current, &mut candidate).unwrap();
        assert_eq!(candidate.blocks[0].revision, 5);
        assert_eq!(candidate.blocks[1].revision, 9);
    }

    #[tokio::test]
    async fn missing_assets_are_a_startup_error() {
        let root =
            std::env::temp_dir().join(format!("logiksmith-no-assets-{}", std::process::id()));
        let error = start_web_server_with_assets(
            store(),
            WebConfig::new("127.0.0.1".parse::<IpAddr>().unwrap(), 1).unwrap(),
            &root,
        )
        .await;
        assert!(error.is_err());
    }

    #[tokio::test]
    async fn static_assets_and_snapshot_are_served() {
        let root = std::env::temp_dir().join(format!("logiksmith-assets-{}", std::process::id()));
        let _ = fs::create_dir_all(&root);
        fs::write(root.join("index.html"), "dashboard").unwrap();
        // Port zero is useful for an isolated test listener; file-based
        // configuration rejects it through `WebConfig::new`.
        let config = WebConfig {
            listen_ip: "127.0.0.1".parse().unwrap(),
            listen_port: 0,
        };
        let server = start_web_server_with_assets(store(), config, &root)
            .await
            .unwrap();
        let mut stream = tokio::net::TcpStream::connect(server.address)
            .await
            .unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        assert!(String::from_utf8_lossy(&response).contains("dashboard"));
        server.shutdown().await;
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn shutdown_does_not_wait_for_an_open_sse_stream() {
        let root =
            std::env::temp_dir().join(format!("logiksmith-sse-shutdown-{}", std::process::id()));
        let _ = fs::create_dir_all(&root);
        fs::write(root.join("index.html"), "dashboard").unwrap();
        let store = store();
        let server = start_web_server_with_assets(
            store.clone(),
            WebConfig {
                listen_ip: "127.0.0.1".parse().unwrap(),
                listen_port: 0,
            },
            &root,
        )
        .await
        .unwrap();
        let mut stream = tokio::net::TcpStream::connect(server.address)
            .await
            .unwrap();
        stream
            .write_all(b"GET /api/events HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        store.set_connection(crate::diagnostics::ConnectionState::Connected);
        let mut response = [0; 256];
        stream.read(&mut response).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), server.shutdown())
            .await
            .expect("shutdown must not wait for an SSE client");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn snapshot_endpoint_returns_the_complete_projection() {
        let root =
            std::env::temp_dir().join(format!("logiksmith-api-assets-{}", std::process::id()));
        let _ = fs::create_dir_all(&root);
        fs::write(root.join("index.html"), "dashboard").unwrap();
        let store = store();
        store.set_connection(crate::diagnostics::ConnectionState::Connected);
        let server = start_web_server_with_assets(
            store,
            WebConfig {
                listen_ip: "127.0.0.1".parse().unwrap(),
                listen_port: 0,
            },
            &root,
        )
        .await
        .unwrap();
        let response = raw_get(server.address, "/api/snapshot").await;
        assert!(response.contains("\"revision\":1"));
        assert!(response.contains("\"connection\":{"));
        assert!(response.contains("\"write\":{"));
        assert!(response.contains("\"status\":\"idle\""));
        assert!(response.contains("\"telegrams\":[]"));
        assert!(response.contains("\"logs\":[]"));
        server.shutdown().await;
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn events_replay_and_resync_when_journal_is_too_old() {
        let root =
            std::env::temp_dir().join(format!("logiksmith-sse-assets-{}", std::process::id()));
        let _ = fs::create_dir_all(&root);
        fs::write(root.join("index.html"), "dashboard").unwrap();
        let store = store();
        store.set_connection(crate::diagnostics::ConnectionState::Connected);
        let server = start_web_server_with_assets(
            store.clone(),
            WebConfig {
                listen_ip: "127.0.0.1".parse().unwrap(),
                listen_port: 0,
            },
            &root,
        )
        .await
        .unwrap();
        let replay = raw_get_prefix(server.address, "/api/events?since=0").await;
        assert!(replay.contains("event: update"));
        assert!(replay.contains("\"revision\":1"));
        for _ in 0..JOURNAL_CAPACITY + 1 {
            store.record_log("info", "test", "event", std::collections::BTreeMap::new());
        }
        let resync = raw_get_prefix(server.address, "/api/events?since=0").await;
        assert!(resync.contains("event: resync"));
        server.shutdown().await;
        let _ = fs::remove_dir_all(root);
    }

    fn simulation_runtime(source: &str) -> crate::AutomationRuntime {
        simulation_runtime_with_schedules(source, Vec::new())
    }

    fn simulation_runtime_with_schedules(
        source: &str,
        schedules: Vec<crate::AutomationSchedule>,
    ) -> crate::AutomationRuntime {
        crate::build_automation(crate::AutomationDocument {
            signals: Vec::new(),
            blocks: vec![crate::AutomationBlock {
                id: "test".to_owned(),
                revision: 1,
                enabled: true,
                inputs: vec![
                    crate::AutomationEndpoint {
                        name: "wall_switch".to_owned(),
                        dpt: "1.001".to_owned(),
                    },
                    crate::AutomationEndpoint {
                        name: "enabled".to_owned(),
                        dpt: "1.001".to_owned(),
                    },
                ],
                outputs: vec![crate::AutomationEndpoint {
                    name: "test_light".to_owned(),
                    dpt: "1.001".to_owned(),
                }],
                knx_bindings: vec![
                    crate::KnxBinding {
                        endpoint: "wall_switch".to_owned(),
                        group_address: "2/2/52".to_owned(),
                    },
                    crate::KnxBinding {
                        endpoint: "enabled".to_owned(),
                        group_address: "2/2/53".to_owned(),
                    },
                    crate::KnxBinding {
                        endpoint: "test_light".to_owned(),
                        group_address: "2/3/52".to_owned(),
                    },
                ],
                signal_bindings: Vec::new(),
                source: source.to_owned(),
                schedules,
            }],
        })
        .unwrap()
    }

    async fn simulation_actor(
        mut receiver: mpsc::Receiver<crate::SimulationRequest>,
        runtime: crate::AutomationRuntime,
        active_revision: u64,
    ) {
        let core_runtime = Runtime::new(runtime.core_config.clone());
        while let Some(request) = receiver.recv().await {
            let crate::SimulationRequest { payload, reply } = request;
            let outcome = if payload.expected_logic_revision != active_revision {
                crate::SimulationOutcome::Conflict {
                    current_revision: active_revision,
                }
            } else {
                let block_id = payload.block_id.parse().expect("known block ID");
                let block = runtime.block(&block_id).expect("known block");
                match crate::simulation_scenario(payload.clone(), block) {
                    Err(errors) => crate::SimulationOutcome::Invalid(errors),
                    Ok(scenario) => match core_runtime
                        .simulate_input(&block_id, scenario)
                    {
                        Ok(execution) => crate::SimulationOutcome::Complete(
                            crate::diagnostics::simulation_response_for_block(
                                &execution,
                                1,
                                active_revision,
                                &runtime,
                            ),
                        ),
                        Err(logiksmith_core::RuntimeSimulationError::Block { error, .. }) => {
                            crate::SimulationOutcome::Invalid(crate::simulation_error_fields(
                                &error, &payload,
                            ))
                        }
                        Err(logiksmith_core::RuntimeSimulationError::UnknownBlock(_)) => {
                            crate::SimulationOutcome::NotFound
                        }
                    },
                }
            };
            let _ = reply.send(outcome);
        }
    }

    fn interval_schedule() -> crate::AutomationSchedule {
        crate::AutomationSchedule {
            name: "heartbeat".to_owned(),
            enabled: true,
            kind: "interval".to_owned(),
            at: None,
            every: Some("60s".to_owned()),
            offset: Some("0s".to_owned()),
            anchor: None,
            weekdays: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn simulation_host_config(runtime: crate::AutomationRuntime) -> crate::RuntimeConfig {
        crate::RuntimeConfig {
            config_path: std::env::temp_dir().join("logiksmith-web-test-config.toml"),
            automation_path: std::env::temp_dir().join("logiksmith-web-test-automation.toml"),
            automation_revision: 0,
            automation: runtime,
            connection: crate::ConnectionConfig {
                gateway_ip: "192.0.2.1".parse().unwrap(),
                gateway_port: 3671,
                local_ip: None,
            },
            bridge: crate::BridgeConfig {
                python: "/bin/sh".into(),
            },
            logging: crate::LoggingConfig {
                level: tracing_subscriber::filter::LevelFilter::OFF,
                bridge_level: tracing_subscriber::filter::LevelFilter::OFF,
            },
            web: WebConfig {
                listen_ip: "127.0.0.1".parse().unwrap(),
                listen_port: 0,
            },
        }
    }

    #[tokio::test]
    async fn dedicated_schedule_routes_use_schedule_specific_json_contract() {
        let root = std::env::temp_dir().join(format!(
            "logiksmith-schedule-api-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("index.html"), "dashboard").unwrap();
        let runtime = simulation_runtime_with_schedules(
            "function handle(event) return nil end",
            vec![interval_schedule()],
        );
        let store = DiagnosticStore::new(&runtime, root.join("automation.toml"), 1);
        let config = simulation_host_config(runtime.clone());
        let mut core_runtime = Runtime::new(runtime.core_config.clone());
        core_runtime
            .initialise_schedules(
                logiksmith_core::ClockSample {
                    monotonic_ms: logiksmith_core::MonotonicMs(0),
                    utc_unix_ms: Some(0),
                },
                runtime.structural_revision,
            )
            .unwrap();
        let (sender, mut receiver) = mpsc::channel(4);
        let actor_store = store.clone();
        let actor_config = config.clone();
        let actor = tokio::spawn(async move {
            while let Some(request) = receiver.recv().await {
                crate::host::apply_simulation(&core_runtime, &actor_store, &actor_config, request);
            }
        });
        let server = start_web_server_with_assets_and_activation(
            store,
            config.web,
            &root,
            None,
            Some(sender),
        )
        .await
        .unwrap();

        // This request intentionally contains no generic input scenario,
        // source-hash revision, state, or pending-timer fields.
        let (status, preview) = raw_post_path(
            server.address,
            "/api/schedules/preview",
            serde_json::json!({
                "block_id": "test",
                "schedule": "heartbeat",
                "after_utc_ms": 0,
                "count": 2
            }),
        )
        .await;
        assert_eq!(status, 200);
        let occurrences = preview["occurrences"].as_array().unwrap();
        assert_eq!(occurrences.len(), 2);
        assert_eq!(occurrences[0]["utc_ms"], 60_000);
        assert_eq!(occurrences[1]["utc_ms"], 120_000);

        let (status, result) = raw_post_path(
            server.address,
            "/api/schedules/simulate",
            serde_json::json!({
                "block_id": "test",
                "schedule": "heartbeat",
                "occurrence_at_utc_ms": occurrences[0]["utc_ms"],
                "expected_revision": "1",
                "expected_structural_revision": runtime.structural_revision.to_string()
            }),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(result["block_id"], "test");
        assert_eq!(result["logic_revision"], "1");
        assert_eq!(result["trigger"]["type"], "schedule");
        assert_eq!(result["trigger"]["name"], "heartbeat");

        let (status, result) = raw_post_path(
            server.address,
            "/api/schedules/preview",
            serde_json::json!({
                "block_id": "test",
                "schedule": "does_not_exist",
                "after_utc_ms": 0,
                "count": 2
            }),
        )
        .await;
        assert_eq!(status, 404);
        assert_eq!(result["error"], "unknown schedule");

        let (status, result) = raw_post_path(
            server.address,
            "/api/schedules/simulate",
            serde_json::json!({
                "block_id": "test",
                "schedule": "does_not_exist",
                "occurrence_at_utc_ms": 60_000,
                "expected_revision": "1",
                "expected_structural_revision": runtime.structural_revision.to_string()
            }),
        )
        .await;
        assert_eq!(status, 404);
        assert_eq!(result["error"], "unknown schedule");

        let (status, result) = raw_post_path(
            server.address,
            "/api/schedules/simulate",
            serde_json::json!({
                "block_id": "test",
                "schedule": "heartbeat",
                "occurrence_at_utc_ms": 60_000,
                "expected_revision": "0",
                "expected_structural_revision": runtime.structural_revision.to_string()
            }),
        )
        .await;
        assert_eq!(status, 409);
        assert_eq!(result["current_revision"], "1");
        assert_eq!(
            result["current_structural_revision"],
            runtime.structural_revision.to_string()
        );
        assert!(result.get("current_logic_revision").is_none());

        let (status, result) = raw_post_path(
            server.address,
            "/api/schedules/simulate",
            serde_json::json!({
                "block_id": "test",
                "schedule": "heartbeat",
                "occurrence_at_utc_ms": 60_000,
                "expected_revision": "1",
                "expected_structural_revision": "0"
            }),
        )
        .await;
        assert_eq!(status, 409);
        assert_eq!(result["current_revision"], "1");
        assert_eq!(
            result["current_structural_revision"],
            runtime.structural_revision.to_string()
        );
        assert!(result.get("current_logic_revision").is_none());

        server.shutdown().await;
        actor.abort();
        let _ = actor.await;
        let _ = fs::remove_dir_all(root);
    }

    fn simulation_payload_for_revision(revision: u64) -> serde_json::Value {
        serde_json::json!({
            "block_id": "test",
            "expected_logic_revision": revision.to_string(),
            "trigger": {
                "endpoint": "wall_switch",
                "value": { "kind": "bool", "value": true },
                "previous": { "kind": "bool", "value": false }
            },
            "inputs": [
                { "endpoint": "wall_switch", "value": { "kind": "bool", "value": true }, "valid": true, "age_ms": 0 },
                { "endpoint": "enabled", "value": null, "valid": false, "age_ms": null }
            ]
        })
    }

    fn simulation_payload() -> serde_json::Value {
        simulation_payload_for_revision(7)
    }

    async fn raw_post(
        address: std::net::SocketAddr,
        body: serde_json::Value,
    ) -> (u16, serde_json::Value) {
        raw_post_path(address, "/api/simulate", body).await
    }

    async fn raw_post_path(
        address: std::net::SocketAddr,
        path: &str,
        body: serde_json::Value,
    ) -> (u16, serde_json::Value) {
        let body = body.to_string();
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(
                format!(
                    "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8(response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let status = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap()
            .parse()
            .unwrap();
        (status, serde_json::from_str(body).unwrap())
    }

    #[tokio::test]
    async fn simulation_endpoint_returns_result_without_diagnostic_mutation() {
        let root = std::env::temp_dir().join(format!(
            "logiksmith-simulation-api-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("index.html"), "dashboard").unwrap();
        let mut runtime = simulation_runtime(
            "function handle(event, input)\n  return { outputs = { test_light = event.rising } }\nend",
        );
        runtime.document_revision = 7;
        let store = DiagnosticStore::new(&runtime, root.join("automation.toml"), 7);
        let before = store.snapshot();
        let (sender, receiver) = mpsc::channel(4);
        let actor = tokio::spawn(simulation_actor(receiver, runtime.clone(), 7));
        let server = start_web_server_with_assets_and_activation(
            store.clone(),
            WebConfig {
                listen_ip: "127.0.0.1".parse().unwrap(),
                listen_port: 0,
            },
            &root,
            None,
            Some(sender),
        )
        .await
        .unwrap();

        let (status, result) = raw_post(server.address, simulation_payload()).await;
        assert_eq!(status, 200);
        assert_eq!(result["logic_revision"], "7");
        assert_eq!(result["status"], "succeeded");
        assert_eq!(result["trigger"]["rising"], true);
        assert_eq!(result["effects"][0]["endpoint"], "test_light");
        assert_eq!(result["effects"][0]["destination"], "2/3/52");
        assert_eq!(store.snapshot(), before);

        let (status, result) = raw_post(
            server.address,
            serde_json::json!({ "expected_logic_revision": 6 }),
        )
        .await;
        assert_eq!(status, 422);
        assert!(result["errors"].is_array());

        let (status, result) = raw_post(server.address, {
            let mut payload = simulation_payload();
            payload["expected_logic_revision"] = serde_json::json!("6");
            payload
        })
        .await;
        assert_eq!(status, 409);
        assert_eq!(result["current_logic_revision"], "7");

        server.shutdown().await;
        actor.abort();
        let _ = actor.await;
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn simulation_round_trips_large_logic_revision_as_a_string() {
        let root = std::env::temp_dir().join(format!(
            "logiksmith-simulation-large-revision-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("index.html"), "dashboard").unwrap();
        let runtime = simulation_runtime(
            "function handle(event, input)\n  return { outputs = { test_light = event.rising } }\nend",
        );
        let revision = 1;
        let store = DiagnosticStore::new(&runtime, root.join("automation.toml"), 7);
        let (sender, receiver) = mpsc::channel(4);
        let actor = tokio::spawn(simulation_actor(receiver, runtime.clone(), revision));
        let server = start_web_server_with_assets_and_activation(
            store,
            WebConfig {
                listen_ip: "127.0.0.1".parse().unwrap(),
                listen_port: 0,
            },
            &root,
            None,
            Some(sender),
        )
        .await
        .unwrap();

        let snapshot = raw_get(server.address, "/api/snapshot").await;
        assert!(snapshot.contains("\"active_logic_revision\":\"1\""));
        let (status, result) =
            raw_post(server.address, simulation_payload_for_revision(revision)).await;
        assert_eq!(status, 200);
        assert_eq!(result["logic_revision"], revision.to_string());

        server.shutdown().await;
        actor.abort();
        let _ = actor.await;
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn simulation_endpoint_returns_zero_effect_and_contained_failure() {
        let root = std::env::temp_dir().join(format!(
            "logiksmith-simulation-outcomes-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("index.html"), "dashboard").unwrap();
        let mut runtime = simulation_runtime("function handle(event) return nil end");
        runtime.document_revision = 7;
        let store = DiagnosticStore::new(&runtime, root.join("automation.toml"), 7);
        let (sender, receiver) = mpsc::channel(4);
        let actor = tokio::spawn(simulation_actor(receiver, runtime.clone(), 7));
        let server = start_web_server_with_assets_and_activation(
            store,
            WebConfig {
                listen_ip: "127.0.0.1".parse().unwrap(),
                listen_port: 0,
            },
            &root,
            None,
            Some(sender),
        )
        .await
        .unwrap();
        let (status, result) = raw_post(server.address, simulation_payload()).await;
        assert_eq!(status, 200);
        assert_eq!(result["status"], "succeeded");
        assert_eq!(result["effects"].as_array().unwrap().len(), 0);
        server.shutdown().await;
        actor.abort();
        let _ = actor.await;

        let root = std::env::temp_dir().join(format!(
            "logiksmith-simulation-failure-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("index.html"), "dashboard").unwrap();
        let mut runtime = simulation_runtime("function handle(event) error('contained boom') end");
        runtime.document_revision = 7;
        let store = DiagnosticStore::new(&runtime, root.join("automation.toml"), 7);
        let (sender, receiver) = mpsc::channel(4);
        let actor = tokio::spawn(simulation_actor(receiver, runtime.clone(), 7));
        let server = start_web_server_with_assets_and_activation(
            store,
            WebConfig {
                listen_ip: "127.0.0.1".parse().unwrap(),
                listen_port: 0,
            },
            &root,
            None,
            Some(sender),
        )
        .await
        .unwrap();
        let (status, result) = raw_post(server.address, simulation_payload()).await;
        assert_eq!(status, 200);
        assert_eq!(result["status"], "failed");
        assert_eq!(result["effects"].as_array().unwrap().len(), 0);
        assert_eq!(result["error"]["category"], "runtime");
        server.shutdown().await;
        actor.abort();
        let _ = actor.await;
        let _ = fs::remove_dir_all(root);
    }

    async fn raw_get(address: std::net::SocketAddr, path: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8(response).unwrap()
    }

    async fn raw_get_prefix(address: std::net::SocketAddr, path: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = vec![0; 4096];
        let bytes = tokio::time::timeout(Duration::from_secs(1), stream.read(&mut response))
            .await
            .unwrap()
            .unwrap();
        String::from_utf8_lossy(&response[..bytes]).into_owned()
    }
