use logiksmith_desktop::{BridgeCommand, HostError, load_config, run_with_bridge};
use std::{
    ffi::OsString,
    fs,
    net::TcpListener,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::{Duration, sleep, timeout},
};

fn temporary_path(suffix: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("logiksmith-desktop-{stamp}-{suffix}"))
}

#[tokio::test]
async fn fake_bridge_drives_lua_outputs() {
    let config_path = temporary_path("config.toml");
    let automation_path = temporary_path("automation.toml");
    let marker_path = temporary_path("writes.log");
    let web_port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    fs::write(
        &config_path,
        format!(
            r#"
[knx]
connection_type = "tunneling"
gateway_ip = "192.0.2.1"
gateway_port = 3671

[bridge]
python = "/bin/sh"

[time]
timezone = "UTC"

[logging]
level = "off"
bridge_level = "off"

[web]
listen_ip = "127.0.0.1"
listen_port = {web_port}
"#
        ),
    )
    .unwrap();
    fs::write(
        &automation_path,
        r#"
[[blocks]]
id = "test"
enabled = true
source = '''
function handle(event, input)
  if event.input == "wall_switch" and event.value == true then
    return { outputs = { test_light = input.dimmer_level == 42 } }
  elseif event.input == "dimmer_level" then
    return { outputs = { dimmer_output = event.value } }
  end
end
'''

[[blocks.inputs]]
name = "wall_switch"
dpt = "1.001"

[[blocks.inputs]]
name = "dimmer_level"
dpt = "5.001"

[[blocks.outputs]]
name = "test_light"
dpt = "1.001"

[[blocks.outputs]]
name = "dimmer_output"
dpt = "5.001"

[[blocks.knx_bindings]]
endpoint = "wall_switch"
group_address = "2/2/52"

[[blocks.knx_bindings]]
endpoint = "dimmer_level"
group_address = "2/2/53"

[[blocks.knx_bindings]]
endpoint = "test_light"
group_address = "2/3/52"

[[blocks.knx_bindings]]
endpoint = "dimmer_output"
group_address = "2/3/53"
"#,
    )
    .unwrap();

    let marker = marker_path.to_str().unwrap();
    let fixture = format!(
        r#"set -eu
printf '%s\n' '{{"v":1,"type":"bridge_hello","bridge":"xknx","bridge_version":"0.1.0","xknx_version":"test"}}'
while IFS= read -r line; do
  case "$line" in
    *'"type":"configure"'*)
      printf '%s\n' '{{"v":1,"type":"ready","transport":"knxip_tunneling","gateway":"192.0.2.1"}}'
      printf '%s\n' '{{"v":1,"type":"knx_event","source":"1.1.42","destination":"2/2/53","service":"group_value_response","dpt":{{"major":5,"subtype":1}},"value":{{"kind":"percent","value":42}}}}'
      printf '%s\n' '{{"v":1,"type":"knx_event","source":"1.1.42","destination":"2/2/52","service":"group_value_write","dpt":{{"major":1,"subtype":1}},"value":{{"kind":"bool","value":true}}}}'
      printf '%s\n' '{{"v":1,"type":"knx_event","source":"1.1.42","destination":"2/2/53","service":"group_value_write","dpt":{{"major":5,"subtype":1}},"value":{{"kind":"percent","value":42}}}}'
      ;;
    *'"type":"knx_write"'*'"value":{{"kind":"bool","value":true}}'*)
      printf '%s\n' on >> '{marker}'
      ;;
    *'"type":"knx_write"'*'"value":{{"kind":"percent","value":42}}'*)
      printf '%s\n' percent >> '{marker}'
      printf '%s\n' '{{"v":1,"type":"fatal","code":"test_complete","message":"fake bridge observed both writes"}}'
      exit 0
      ;;
    *'"type":"shutdown"'*) exit 0 ;;
  esac
done
"#
    );

    let config = load_config(&config_path, &automation_path).unwrap();
    let result = run_with_bridge(
        config,
        BridgeCommand::new(
            "/bin/sh",
            vec![OsString::from("-c"), OsString::from(fixture)],
        ),
    )
    .await;
    assert!(matches!(
        result,
        Err(HostError::BridgeFatal { code, .. }) if code == "test_complete"
    ));

    let writes = fs::read_to_string(&marker_path).unwrap();
    assert_eq!(writes, "on\npercent\n");
    let _ = fs::remove_file(config_path);
    let _ = fs::remove_file(automation_path);
    let _ = fs::remove_file(marker_path);
}

async fn read_snapshot(port: u16) -> Option<serde_json::Value> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
    stream
        .write_all(b"GET /api/snapshot HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.ok()?;
    let body = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| &response[index + 4..])?;
    serde_json::from_slice(body).ok()
}

#[tokio::test]
async fn fake_bridge_execution_inspector_captures_transitions_and_failure() {
    let config_path = temporary_path("inspector-config.toml");
    let automation_path = temporary_path("inspector-automation.toml");
    let marker_path = temporary_path("inspector-writes.log");
    let web_port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    fs::write(
        &config_path,
        format!(
            r#"
[knx]
connection_type = "tunneling"
gateway_ip = "192.0.2.1"
gateway_port = 3671

[bridge]
python = "/bin/sh"

[time]
timezone = "UTC"

[logging]
level = "off"
bridge_level = "off"

[web]
listen_ip = "127.0.0.1"
listen_port = {web_port}
"#
        ),
    )
    .unwrap();
    fs::write(
        &automation_path,
        r#"
[[blocks]]
id = "test"
enabled = true
source = '''
function handle(event, input, meta)
  if event.falling then error("contained boom") end
  if event.input == "wall_switch" and event.rising and meta.enabled.valid and input.enabled == true then
    return { outputs = { test_light = true } }
  end
end
'''

[[blocks.inputs]]
name = "wall_switch"
dpt = "1.001"

[[blocks.inputs]]
name = "enabled"
dpt = "1.001"

[[blocks.outputs]]
name = "test_light"
dpt = "1.001"

[[blocks.knx_bindings]]
endpoint = "wall_switch"
group_address = "2/2/52"

[[blocks.knx_bindings]]
endpoint = "enabled"
group_address = "2/2/53"

[[blocks.knx_bindings]]
endpoint = "test_light"
group_address = "2/3/52"
"#,
    )
    .unwrap();

    let marker = marker_path.to_str().unwrap();
    let fixture = format!(
        r#"set -eu
printf '%s\n' '{{"v":1,"type":"bridge_hello","bridge":"xknx","bridge_version":"0.1.0","xknx_version":"test"}}'
writes=0
while IFS= read -r line; do
  case "$line" in
    *'"type":"configure"'*)
      printf '%s\n' '{{"v":1,"type":"ready","transport":"knxip_tunneling","gateway":"192.0.2.1"}}'
      printf '%s\n' '{{"v":1,"type":"knx_event","source":"1.1.42","destination":"2/2/53","service":"group_value_response","dpt":{{"major":1,"subtype":1}},"value":{{"kind":"bool","value":true}}}}'
      printf '%s\n' '{{"v":1,"type":"knx_event","source":"1.1.42","destination":"2/2/52","service":"group_value_write","dpt":{{"major":1,"subtype":1}},"value":{{"kind":"bool","value":false}}}}'
      printf '%s\n' '{{"v":1,"type":"knx_event","source":"1.1.42","destination":"2/2/52","service":"group_value_write","dpt":{{"major":1,"subtype":1}},"value":{{"kind":"bool","value":true}}}}'
      printf '%s\n' '{{"v":1,"type":"knx_event","source":"1.1.42","destination":"2/2/52","service":"group_value_write","dpt":{{"major":1,"subtype":1}},"value":{{"kind":"bool","value":true}}}}'
      printf '%s\n' '{{"v":1,"type":"knx_event","source":"1.1.42","destination":"2/2/52","service":"group_value_write","dpt":{{"major":1,"subtype":1}},"value":{{"kind":"bool","value":false}}}}'
      printf '%s\n' '{{"v":1,"type":"knx_event","source":"1.1.42","destination":"2/2/52","service":"group_value_write","dpt":{{"major":1,"subtype":1}},"value":{{"kind":"bool","value":true}}}}'
      ;;
    *'"type":"knx_write"'*'"value":{{"kind":"bool","value":true}}'*)
      writes=$((writes + 1))
      printf '%s\n' on >> '{marker}'
      if [ "$writes" -eq 2 ]; then sleep 1; printf '%s\n' '{{"v":1,"type":"fatal","code":"test_complete","message":"inspector complete"}}'; exit 0; fi
      ;;
    *'"type":"shutdown"'*) exit 0 ;;
  esac
done
"#
    );

    let config = load_config(&config_path, &automation_path).unwrap();
    let task = tokio::spawn(run_with_bridge(
        config,
        BridgeCommand::new(
            "/bin/sh",
            vec![OsString::from("-c"), OsString::from(fixture)],
        ),
    ));
    let snapshot = timeout(Duration::from_secs(5), async {
        loop {
            if let Some(snapshot) = read_snapshot(web_port).await
                && snapshot["logic"]["executions"]
                    .as_array()
                    .is_some_and(|executions| executions.len() == 5)
            {
                break snapshot;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fake bridge should produce five execution records");
    let result = task.await.unwrap();
    assert!(matches!(
        result,
        Err(HostError::BridgeFatal { code, .. }) if code == "test_complete"
    ));

    let executions = snapshot["logic"]["executions"].as_array().unwrap();
    assert_eq!(executions.len(), 5);
    assert!(snapshot["logic"].get("last_execution").is_none());
    assert!(snapshot["logic"].get("recent_effects").is_none());
    for execution in executions {
        assert!(execution["execution_id"].as_u64().is_some());
        assert!(execution["logic_revision"].as_str().is_some());
        assert!(execution["duration_us"].as_u64().is_some());
        assert_eq!(execution["inputs"].as_array().unwrap().len(), 2);
        assert_eq!(execution["inputs"][1]["value"]["value"], true);
        assert_eq!(execution["inputs"][1]["valid"], true);
        assert!(execution["inputs"][1]["age_ms"].as_u64().is_some());
    }

    let recovery = &executions[0];
    assert_eq!(recovery["status"], "succeeded");
    assert_eq!(recovery["trigger"]["endpoint"], "wall_switch");
    assert_eq!(recovery["trigger"]["previous"]["value"], false);
    assert_eq!(recovery["trigger"]["changed"], true);
    assert_eq!(recovery["trigger"]["rising"], true);
    assert_eq!(recovery["trigger"]["falling"], false);
    assert_eq!(recovery["inputs"][0]["value"]["value"], true);
    assert_eq!(recovery["effects"][0]["destination"], "2/3/52");

    let failure = &executions[1];
    assert_eq!(failure["status"], "failed");
    assert_eq!(failure["trigger"]["falling"], true);
    assert_eq!(failure["inputs"][0]["value"]["value"], false);
    assert_eq!(failure["effects"].as_array().unwrap().len(), 0);
    assert_eq!(failure["error"]["category"], "runtime");
    assert!(
        failure["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("contained boom"))
    );

    let repeated = &executions[2];
    assert_eq!(repeated["status"], "succeeded");
    assert_eq!(repeated["trigger"]["previous"]["value"], true);
    assert_eq!(repeated["trigger"]["changed"], false);
    assert_eq!(repeated["trigger"]["rising"], false);
    assert_eq!(repeated["trigger"]["falling"], false);
    assert_eq!(repeated["effects"].as_array().unwrap().len(), 0);

    let first_rising = &executions[3];
    assert_eq!(first_rising["status"], "succeeded");
    assert_eq!(first_rising["trigger"]["previous"]["value"], false);
    assert_eq!(first_rising["trigger"]["changed"], true);
    assert_eq!(first_rising["trigger"]["rising"], true);
    assert_eq!(first_rising["trigger"]["falling"], false);
    assert_eq!(first_rising["inputs"][0]["value"]["value"], true);
    assert_eq!(first_rising["effects"][0]["destination"], "2/3/52");

    let initial_false = &executions[4];
    assert_eq!(initial_false["status"], "succeeded");
    assert!(initial_false["trigger"]["previous"].is_null());
    assert_eq!(initial_false["trigger"]["changed"], false);
    assert_eq!(initial_false["trigger"]["rising"], false);
    assert_eq!(initial_false["trigger"]["falling"], false);
    assert_eq!(initial_false["inputs"][0]["value"]["value"], false);
    assert_eq!(initial_false["effects"].as_array().unwrap().len(), 0);

    let writes = fs::read_to_string(&marker_path).unwrap();
    assert_eq!(writes, "on\non\n");
    let _ = fs::remove_file(config_path);
    let _ = fs::remove_file(automation_path);
    let _ = fs::remove_file(marker_path);
}
