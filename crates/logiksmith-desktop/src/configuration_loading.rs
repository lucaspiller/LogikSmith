pub fn load_config(
    config_path: &Path,
    automation_path: &Path,
) -> Result<RuntimeConfig, ConfigError> {
    load_config_with_bridge_validation(config_path, automation_path, true)
}

/// Loads the browser/simulation runtime configuration without requiring an
/// installed Python bridge executable. KNX details remain parsed so the same
/// local configuration works when normal mode resumes.
pub fn load_simulation_config(
    config_path: &Path,
    automation_path: &Path,
) -> Result<RuntimeConfig, ConfigError> {
    load_config_with_bridge_validation(config_path, automation_path, false)
}

fn load_config_with_bridge_validation(
    config_path: &Path,
    automation_path: &Path,
    validate_bridge: bool,
) -> Result<RuntimeConfig, ConfigError> {
    let source = fs::read_to_string(config_path).map_err(|source| ConfigError::Read {
        path: config_path.to_path_buf(),
        source,
    })?;
    let raw: RawConfig = toml::from_str(&source)?;
    let host_limits = HostLimits::from_environment()
        .map_err(|error| field("runtime.profile", error.to_string()))?;
    let automation_source =
        fs::read(automation_path).map_err(|source| ConfigError::AutomationRead {
            path: automation_path.to_path_buf(),
            source,
        })?;
    let automation_text = String::from_utf8_lossy(&automation_source);
    if let Ok(value) = toml::from_str::<toml::Value>(&automation_text) {
        let legacy = ["inputs", "outputs", "knx_bindings", "logic"]
            .into_iter()
            .filter(|field| value.get(*field).is_some())
            .map(|field| FieldError {
                path: (*field).to_owned(),
                message: "legacy top-level field must move inside [[blocks]]".to_owned(),
            })
            .collect::<Vec<_>>();
        if !legacy.is_empty() {
            return Err(ConfigError::AutomationInvalid(legacy));
        }
    }
    let stored = toml::from_str::<StoredAutomation>(&automation_text)
        .map_err(ConfigError::AutomationToml)?;
    let mut automation = build_automation_with_limits(
        stored.document,
        host_limits.profile.limits(),
    )
    .map_err(ConfigError::AutomationInvalid)?;
    automation.document_revision = 0;
    let timezone = TimeZoneId::new(&raw.time.timezone)
        .map_err(|error| field("time.timezone", error.to_string()))?;
    let coordinates = match (raw.time.latitude, raw.time.longitude) {
        (Some(latitude), Some(longitude)) => {
            if !(-90.0..=90.0).contains(&latitude) {
                return Err(field("time.latitude", "must be in range -90..=90"));
            }
            if !(-180.0..180.0).contains(&longitude) {
                return Err(field("time.longitude", "must be in range -180..<180"));
            }
            Some(Coordinates {
                latitude,
                longitude,
            })
        }
        (None, None) => None,
        _ => {
            return Err(field(
                "time.latitude",
                "latitude and longitude must be supplied as a pair",
            ));
        }
    };
    let mut site_errors = Vec::new();
    for (block_index, block) in automation.document.blocks.iter().enumerate() {
        for (schedule_index, schedule) in block.schedules.iter().enumerate() {
            if schedule.enabled && schedule.kind == "astronomical" && coordinates.is_none() {
                site_errors.push(FieldError {
                    path: format!("blocks[{block_index}].schedules[{schedule_index}].anchor"),
                    message: "astronomical schedules require [time].latitude and [time].longitude"
                        .to_owned(),
                });
            }
        }
    }
    if !site_errors.is_empty() {
        return Err(ConfigError::AutomationInvalid(site_errors));
    }
    automation.core_config.site = SiteTimeConfig {
        timezone,
        coordinates,
    };
    if raw.knx.connection_type != "tunneling" {
        return Err(field("knx.connection_type", "must be 'tunneling'"));
    }
    if raw.knx.gateway_port == 0 || raw.knx.gateway_port > u16::MAX as u32 {
        return Err(field("knx.gateway_port", "must be in range 1..=65535"));
    }
    let gateway_ip = parse_ip("knx.gateway_ip", &raw.knx.gateway_ip)?;
    let local_ip = raw
        .knx
        .local_ip
        .as_deref()
        .map(|value| parse_ip("knx.local_ip", value))
        .transpose()?;
    if raw.bridge.python.is_empty() {
        return Err(field("bridge.python", "must not be empty"));
    }
    let python = PathBuf::from(&raw.bridge.python);
    if validate_bridge && !python.is_file() {
        return Err(field(
            "bridge.python",
            format!("executable does not exist: {}", python.display()),
        ));
    }
    let listen_ip = parse_ip("web.listen_ip", &raw.web.listen_ip)?;
    if raw.web.listen_port == 0 || raw.web.listen_port > u16::MAX as u32 {
        return Err(field("web.listen_port", "must be in range 1..=65535"));
    }
    if !listen_ip.is_loopback()
        && automation
            .webhook_inputs
            .iter()
            .any(|source| source.bearer_token.is_none())
    {
        return Err(ConfigError::AutomationInvalid(vec![FieldError {
            path: "webhook_inputs".to_owned(),
            message: "every webhook requires bearer_token_env when web.listen_ip is not loopback"
                .to_owned(),
        }]));
    }
    Ok(RuntimeConfig {
        config_path: config_path.to_path_buf(),
        automation_path: automation_path.to_path_buf(),
        automation,
        automation_revision: 0,
        connection: ConnectionConfig {
            gateway_ip,
            gateway_port: raw.knx.gateway_port as u16,
            local_ip,
        },
        bridge: BridgeConfig { python },
        logging: LoggingConfig {
            level: parse_level("logging.level", &raw.logging.level)?,
            bridge_level: parse_level("logging.bridge_level", &raw.logging.bridge_level)?,
        },
        web: WebConfig {
            listen_ip,
            listen_port: raw.web.listen_port as u16,
        },
    })
}
