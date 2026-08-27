use logiksmith_desktop::{BridgeCommand, HostError, load_config, run_with_bridge};
use std::{
    ffi::OsString,
    fs,
    net::TcpListener,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn temporary_path(suffix: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("logiksmith-desktop-{stamp}-{suffix}"))
}

#[tokio::test]
async fn fake_bridge_drives_on_then_timer_off() {
    let config_path = temporary_path("config.toml");
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

[poc]
input_group_address = "2/2/52"
input_dpt = "1.001"
output_group_address = "2/3/52"
output_dpt = "1.001"
off_delay_ms = 30

[bridge]
python = "/bin/sh"

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

    let marker = marker_path.to_str().unwrap();
    let fixture = format!(
        r#"set -eu
printf '%s\n' '{{"v":1,"type":"bridge_hello","bridge":"xknx","bridge_version":"0.1.0","xknx_version":"test"}}'
while IFS= read -r line; do
  case "$line" in
    *'"type":"configure"'*)
      printf '%s\n' '{{"v":1,"type":"ready","transport":"knxip_tunneling","gateway":"192.0.2.1"}}'
      printf '%s\n' '{{"v":1,"type":"knx_event","source":"1.1.42","destination":"2/2/52","service":"group_value_write","dpt":{{"major":1,"subtype":1}},"value":{{"kind":"bool","value":true}}}}'
      ;;
    *'"type":"knx_write"'*'"value":{{"kind":"bool","value":true}}'*)
      printf '%s\n' on >> '{marker}'
      ;;
    *'"type":"knx_write"'*'"value":{{"kind":"bool","value":false}}'*)
      printf '%s\n' off >> '{marker}'
      printf '%s\n' '{{"v":1,"type":"fatal","code":"test_complete","message":"fake bridge observed both writes"}}'
      exit 0
      ;;
    *'"type":"shutdown"'*) exit 0 ;;
  esac
done
"#
    );

    let config = load_config(&config_path).unwrap();
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
    assert_eq!(writes, "on\noff\n");
    let _ = fs::remove_file(config_path);
    let _ = fs::remove_file(marker_path);
}
