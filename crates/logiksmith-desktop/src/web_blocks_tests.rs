#[tokio::test]
async fn block_simulation_route_evaluates_supplied_source_without_store_mutation() {
    let root = std::env::temp_dir().join(format!(
        "logiksmith-block-simulation-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("index.html"), "dashboard").unwrap();
    let runtime = simulation_runtime(
        "function handle(event, input) return { outputs = { test_light = false } } end",
    );
    let store = DiagnosticStore::new(&runtime, root.join("automation.toml"), 1);
    let before = store.snapshot();
    let config = simulation_host_config(runtime.clone());
    let (sender, mut receiver) = mpsc::channel(4);
    let actor_store = store.clone();
    let actor_config = config.clone();
    let core_runtime = Runtime::new(runtime.core_config.clone());
    let actor = tokio::spawn(async move {
        while let Some(request) = receiver.recv().await {
            crate::host::apply_simulation(&core_runtime, &actor_store, &actor_config, request);
        }
    });
    let server = start_web_server_with_assets_and_activation(
        store.clone(),
        config.web,
        &root,
        None,
        Some(sender),
        None,
        std::sync::Arc::new(std::collections::HashMap::new()),
    )
    .await
    .unwrap();
    let source =
        "function handle(event, input) return { outputs = { test_light = true } } end";
    let (status, result) = raw_post_path(
        server.address,
        "/api/blocks/test/simulate",
        serde_json::json!({
            "block_id": "test",
            "source": source,
            "source_fingerprint": crate::source_fingerprint(source),
            "expected_revision": "1",
            "expected_structural_revision": runtime.structural_revision.to_string(),
            "trigger": {
                "endpoint": "wall_switch",
                "value": { "kind": "bool", "value": true },
                "previous": { "kind": "bool", "value": false }
            },
            "inputs": [
                { "endpoint": "wall_switch", "value": { "kind": "bool", "value": true }, "valid": true, "age_ms": 0 },
                { "endpoint": "enabled", "value": null, "valid": false, "age_ms": null }
            ]
        }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(result["status"], "succeeded");
    assert_eq!(result["effects"][0]["endpoint"], "test_light");
    assert_eq!(result["source_fingerprint"], crate::source_fingerprint(source));
    assert_eq!(result["block_revision"], "1");
    assert_eq!(result["structural_revision"], runtime.structural_revision.to_string());
    assert_eq!(store.snapshot(), before);
    server.shutdown().await;
    actor.abort();
    let _ = actor.await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn block_source_and_enabled_routes_persist_and_activate_with_cas() {
    let root = std::env::temp_dir().join(format!(
        "logiksmith-block-mutation-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("index.html"), "dashboard").unwrap();
    let runtime = simulation_runtime(
        "function handle(event, input) return { outputs = { test_light = false } } end",
    );
    let path = root.join("automation.toml");
    fs::write(&path, serialize_automation(&runtime.document, 0).unwrap()).unwrap();
    let store = DiagnosticStore::new(&runtime, path.clone(), 1);
    let config = simulation_host_config(runtime.clone());
    let (sender, mut receiver) = mpsc::channel(4);
    let actor_store = store.clone();
    let actor_config = config.clone();
    let mut core_runtime = Runtime::new(runtime.core_config.clone());
    let actor = tokio::spawn(async move {
        while let Some(request) = receiver.recv().await {
            crate::host::apply_activation(
                &mut core_runtime,
                &actor_store,
                &actor_config,
                request,
            );
        }
    });
    let server = start_web_server_with_assets_and_activation(
        store.clone(),
        config.web,
        &root,
        Some(sender),
        None,
        None,
        std::sync::Arc::new(std::collections::HashMap::new()),
    )
    .await
    .unwrap();
    let structural = runtime.structural_revision.to_string();
    let source = "function handle(event, input) return { outputs = { test_light = true } } end";
    let (status, result) = raw_put_path(
        server.address,
        "/api/blocks/test/source",
        serde_json::json!({
            "source": source,
            "source_fingerprint": crate::source_fingerprint(source),
            "expected_revision": "1",
            "expected_structural_revision": structural
        }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(result["active_revision"], "2");
    assert_eq!(result["saved_revision"], "2");
    assert_eq!(result["active_logic_revision"], "2");
    assert_eq!(result["saved_logic_revision"], "2");
    assert_eq!(result["active_enabled"], true);
    assert_eq!(result["active_structural_revision"], structural);
    assert_eq!(result["saved_structural_revision"], structural);
    assert_eq!(result["restart_required"], false);
    assert_eq!(result["source_fingerprint"], crate::source_fingerprint(source));
    let (saved, _) = load_automation(&path).unwrap();
    assert_eq!(saved.blocks[0].source, source);

    let (status, result) = raw_put_path(
        server.address,
        "/api/blocks/test/enabled",
        serde_json::json!({
            "enabled": false,
            "expected_revision": "2",
            "expected_structural_revision": runtime.structural_revision.to_string()
        }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(result["active_revision"], "3");
    assert_eq!(result["active_logic_revision"], "3");
    assert_eq!(result["active_enabled"], false);
    assert_eq!(store.snapshot().blocks[0].active_enabled, false);

    let (status, result) = raw_put_path(
        server.address,
        "/api/blocks/test/enabled",
        serde_json::json!({
            "enabled": true,
            "expected_revision": "2",
            "expected_structural_revision": runtime.structural_revision.to_string()
        }),
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(result["current_revision"], "3");

    server.shutdown().await;
    actor.abort();
    let _ = actor.await;
    let _ = fs::remove_dir_all(root);
}

async fn raw_put_path(
    address: std::net::SocketAddr,
    path: &str,
    body: serde_json::Value,
) -> (u16, serde_json::Value) {
    let body = body.to_string();
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream
        .write_all(
            format!(
                "PUT {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
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
