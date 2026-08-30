/// Validates a complete automation document and constructs its core and
/// desktop-side KNX routing maps.
pub fn build_automation(
    document: AutomationDocument,
) -> Result<AutomationRuntime, Vec<FieldError>> {
    let mut errors = Vec::new();
    let mut signal_dpts = HashMap::new();
    let mut signals = Vec::new();
    if document.signals.len() > MAX_SIGNALS {
        errors.push(FieldError {
            path: "signals".to_owned(),
            message: format!("must contain at most {MAX_SIGNALS} signals"),
        });
    }
    for (signal_index, signal) in document.signals.iter().enumerate() {
        let path = format!("signals[{signal_index}]");
        let name = match endpoint_name(&format!("{path}.name"), &signal.name) {
            Ok(name) => name,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        if name.to_string() != signal.name {
            errors.push(FieldError {
                path: format!("{path}.name"),
                message: "must use canonical identifier form".to_owned(),
            });
            continue;
        }
        let dpt = match parse_dpt(&format!("{path}.dpt"), &signal.dpt) {
            Ok(dpt) => dpt,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        if signal_dpts.insert(signal.name.clone(), dpt).is_some() {
            errors.push(FieldError {
                path: format!("{path}.name"),
                message: "must be unique".to_owned(),
            });
            continue;
        }
        signals.push(SignalRuntime {
            name: signal.name.clone(),
            dpt,
        });
    }
    if document.blocks.is_empty() {
        errors.push(FieldError {
            path: "blocks".to_owned(),
            message: "must contain at least one block".to_owned(),
        });
    } else if document.blocks.len() > MAX_BLOCKS {
        errors.push(FieldError {
            path: "blocks".to_owned(),
            message: format!("must contain at most {MAX_BLOCKS} blocks"),
        });
    }
    let mut block_ids = HashSet::new();
    let mut blocks = Vec::new();
    let mut address_to_inputs: HashMap<GroupAddress, Vec<BlockInputBinding>> = HashMap::new();
    let mut output_to_address = HashMap::new();
    let mut signal_to_inputs: HashMap<String, Vec<BlockSignalInputBinding>> = HashMap::new();
    let mut output_to_signal = HashMap::new();
    let mut address_dpts = HashMap::new();
    let mut address_origins: HashMap<GroupAddress, (String, String)> = HashMap::new();
    let mut signal_producers: HashMap<String, (String, String)> = HashMap::new();
    let mut signal_binding_count = 0usize;

    for (block_index, block) in document.blocks.iter().enumerate() {
        let block_path = format!("blocks[{block_index}]");
        let id = match block.id.parse::<logiksmith_core::BlockId>() {
            Ok(id) => id,
            Err(error) => {
                errors.push(FieldError {
                    path: format!("{block_path}.id"),
                    message: error.to_string(),
                });
                continue;
            }
        };
        if !block_ids.insert(id.clone()) {
            errors.push(FieldError {
                path: format!("{block_path}.id"),
                message: "must be unique".to_owned(),
            });
            continue;
        }
        let mut endpoint_dpts = HashMap::new();
        let mut endpoints = Vec::new();
        let mut endpoint_directions = HashMap::new();
        let mut seen_names = HashSet::new();
        for (direction, declarations, list_name) in [
            (EndpointDirection::Input, &block.inputs, "inputs"),
            (EndpointDirection::Output, &block.outputs, "outputs"),
        ] {
            for (index, declaration) in declarations.iter().enumerate() {
                let path = format!("{block_path}.{list_name}[{index}]");
                let name = match endpoint_name(&format!("{path}.name"), &declaration.name) {
                    Ok(name) => name,
                    Err(error) => {
                        errors.push(error);
                        continue;
                    }
                };
                if !seen_names.insert(name.clone()) {
                    errors.push(FieldError {
                        path: format!("{path}.name"),
                        message: "must be unique within this block".to_owned(),
                    });
                    continue;
                }
                let dpt = match parse_dpt(&format!("{path}.dpt"), &declaration.dpt) {
                    Ok(dpt) => dpt,
                    Err(error) => {
                        errors.push(error);
                        continue;
                    }
                };
                endpoint_dpts.insert(name.clone(), dpt);
                endpoint_directions.insert(name.clone(), direction);
                endpoints.push(Endpoint::new(name, direction, dpt));
            }
        }
        if block.source.is_empty() {
            errors.push(FieldError {
                path: format!("{block_path}.source"),
                message: "must not be empty".to_owned(),
            });
        } else if block.source.len() > MAX_LOGIC_SOURCE_BYTES {
            errors.push(FieldError {
                path: format!("{block_path}.source"),
                message: "must not exceed 65536 bytes".to_owned(),
            });
        }

        let mut endpoint_to_address = HashMap::new();
        let mut endpoint_to_signal = HashMap::new();
        let mut local_addresses = HashSet::new();
        for (index, binding) in block.knx_bindings.iter().enumerate() {
            let path = format!("{block_path}.knx_bindings[{index}]");
            let name = match endpoint_name(&format!("{path}.endpoint"), &binding.endpoint) {
                Ok(name) => name,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            let Some(&dpt) = endpoint_dpts.get(&name) else {
                errors.push(FieldError {
                    path: format!("{path}.endpoint"),
                    message: "must reference an existing endpoint in this block".to_owned(),
                });
                continue;
            };
            let address = match GroupAddress::parse(&binding.group_address) {
                Ok(address) if address.to_string() == binding.group_address => address,
                Ok(_) => {
                    errors.push(FieldError {
                        path: format!("{path}.group_address"),
                        message: "must use canonical main/middle/subgroup form".to_owned(),
                    });
                    continue;
                }
                Err(error) => {
                    errors.push(FieldError {
                        path: format!("{path}.group_address"),
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            if endpoint_to_address.contains_key(&name) {
                errors.push(FieldError {
                    path: format!("{path}.endpoint"),
                    message: "must have exactly one KNX binding".to_owned(),
                });
                continue;
            }
            if !local_addresses.insert(address) {
                errors.push(FieldError {
                    path: format!("{path}.group_address"),
                    message: "may bind only one endpoint within a block".to_owned(),
                });
                continue;
            }
            if let Some(previous) = address_dpts.get(&address)
                && previous != &dpt
            {
                let (previous_block, previous_endpoint) = address_origins
                    .get(&address)
                    .cloned()
                    .unwrap_or_else(|| ("another block".to_owned(), "unknown".to_owned()));
                errors.push(FieldError {
                    path: format!("{path}.group_address"),
                    message: format!(
                        "DPT conflicts with {previous_block}.{previous_endpoint} at {address}"
                    ),
                });
                continue;
            }
            address_dpts.insert(address, dpt);
            address_origins.insert(address, (block.id.clone(), name.to_string()));
            endpoint_to_address.insert(name.clone(), address);
            match endpoint_directions.get(&name) {
                Some(EndpointDirection::Input) => address_to_inputs
                    .entry(address)
                    .or_default()
                    .push(BlockInputBinding {
                        block_id: block.id.clone(),
                        endpoint: name,
                        dpt,
                        address,
                    }),
                Some(EndpointDirection::Output) => {
                    output_to_address.insert((block.id.clone(), name), address);
                }
                None => unreachable!("endpoint DPT implies direction"),
            }
        }
        for (index, binding) in block.signal_bindings.iter().enumerate() {
            signal_binding_count = signal_binding_count.saturating_add(1);
            let path = format!("{block_path}.signal_bindings[{index}]");
            let name = match endpoint_name(&format!("{path}.endpoint"), &binding.endpoint) {
                Ok(name) => name,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            let Some(&dpt) = endpoint_dpts.get(&name) else {
                errors.push(FieldError {
                    path: format!("{path}.endpoint"),
                    message: "must reference an existing endpoint in this block".to_owned(),
                });
                continue;
            };
            let Some(&signal_dpt) = signal_dpts.get(&binding.signal) else {
                errors.push(FieldError {
                    path: format!("{path}.signal"),
                    message: "must reference an existing signal".to_owned(),
                });
                continue;
            };
            if dpt != signal_dpt {
                errors.push(FieldError {
                    path: format!("{path}.signal"),
                    message: format!("DPT conflicts with endpoint {name}"),
                });
                continue;
            }
            if endpoint_to_address.contains_key(&name) || endpoint_to_signal.contains_key(&name) {
                errors.push(FieldError {
                    path: format!("{path}.endpoint"),
                    message: "must have exactly one binding across KNX and signals".to_owned(),
                });
                continue;
            }
            endpoint_to_signal.insert(name.clone(), binding.signal.clone());
            if endpoint_directions.get(&name) == Some(&EndpointDirection::Input) {
                signal_to_inputs
                    .entry(binding.signal.clone())
                    .or_default()
                    .push(BlockSignalInputBinding {
                        block_id: block.id.clone(),
                        endpoint: name,
                        dpt,
                        signal: binding.signal.clone(),
                    });
            } else if endpoint_directions.get(&name) == Some(&EndpointDirection::Output) {
                if let Some((previous_block, previous_endpoint)) =
                    signal_producers.insert(
                        binding.signal.clone(),
                        (block.id.clone(), name.to_string()),
                    )
                {
                    errors.push(FieldError {
                        path: format!("{path}.signal"),
                        message: format!(
                            "must have at most one producer; already produced by {previous_block}.{previous_endpoint}"
                        ),
                    });
                }
                output_to_signal.insert((block.id.clone(), name), binding.signal.clone());
            }
        }
        for endpoint in &endpoints {
            if !endpoint_to_address.contains_key(&endpoint.name)
                && !endpoint_to_signal.contains_key(&endpoint.name)
            {
                let list = match endpoint.direction {
                    EndpointDirection::Input => "inputs",
                    EndpointDirection::Output => "outputs",
                };
                let index = match endpoint.direction {
                    EndpointDirection::Input => block
                        .inputs
                        .iter()
                        .position(|item| item.name == endpoint.name.as_str()),
                    EndpointDirection::Output => block
                        .outputs
                        .iter()
                        .position(|item| item.name == endpoint.name.as_str()),
                };
                errors.push(FieldError {
                    path: index.map_or_else(
                        || format!("{block_path}.{list}"),
                        |index| format!("{block_path}.{list}[{index}].name"),
                    ),
                    message: format!(
                        "endpoint {} must have exactly one binding across KNX and signals",
                        endpoint.name
                    ),
                });
            }
        }
        let schedules = block_schedules(&block.schedules, &block_path, &mut errors);
        let engine_config = EngineConfig::new(endpoints, block.source.clone());
        if let Err(error) = engine_config.validate() {
            errors.push(FieldError {
                path: format!("{block_path}.source"),
                message: error.to_string(),
            });
        }
        blocks.push(BlockRuntime {
            id: block.id.clone(),
            revision: block.revision.max(1),
            enabled: block.enabled,
            engine_config,
            endpoint_to_address,
            endpoint_to_signal,
            endpoint_dpts,
            schedules,
        });
    }
    if signal_binding_count > MAX_SIGNAL_BINDINGS {
        errors.push(FieldError {
            path: "blocks.signal_bindings".to_owned(),
            message: format!("must contain at most {MAX_SIGNAL_BINDINGS} bindings"),
        });
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let core_blocks = blocks
        .iter()
        .map(|block| {
            let mut config = CoreBlockConfig::with_schedules(
                block.id.parse::<BlockId>().expect("validated block ID"),
                block.enabled,
                block.engine_config.endpoints.clone(),
                block.engine_config.logic.source.clone(),
                block.schedules.clone(),
            );
            config.signal_bindings = document
                .blocks
                .iter()
                .find(|candidate| candidate.id == block.id)
                .map(|candidate| {
                    candidate
                        .signal_bindings
                        .iter()
                        .map(|binding| {
                            CoreSignalBinding::new(
                                binding.endpoint.parse().expect("validated endpoint"),
                                SignalName::new(binding.signal.clone())
                                    .expect("validated signal"),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            config
        })
        .collect();
    let core_signals = signals
        .iter()
        .map(|signal| {
            CoreSignalConfig::new(
                SignalName::new(signal.name.clone()).expect("validated signal"),
                signal.dpt,
            )
        })
        .collect();
    let core_config = CoreRuntimeConfig::with_signals(core_blocks, core_signals);
    if let Err(error) = core_config.validate() {
        return Err(vec![core_validation_error(&error, &document)]);
    }
    Ok(AutomationRuntime {
        structural_revision: structural_revision(&document),
        document_revision: 0,
        document,
        signals,
        core_config,
        blocks,
        address_to_inputs,
        output_to_address,
        signal_to_inputs,
        output_to_signal,
        address_dpts,
    })
}

fn core_validation_error(
    error: &logiksmith_core::RuntimeConfigError,
    document: &AutomationDocument,
) -> FieldError {
    let block_path = |id: &logiksmith_core::BlockId| {
        document
            .blocks
            .iter()
            .position(|block| block.id == id.as_str())
            .map(|index| format!("blocks[{index}]"))
            .unwrap_or_else(|| "blocks".to_owned())
    };
    let signal_path = |name: &logiksmith_core::SignalName| {
        document
            .signals
            .iter()
            .position(|signal| signal.name == name.as_str())
            .map(|index| format!("signals[{index}]"))
            .unwrap_or_else(|| "signals".to_owned())
    };
    match error {
        logiksmith_core::RuntimeConfigError::TooManySignals { .. }
        | logiksmith_core::RuntimeConfigError::DuplicateSignal(_)
        | logiksmith_core::RuntimeConfigError::UnsupportedSignalDpt { .. } => FieldError {
            path: signal_path(match error {
                logiksmith_core::RuntimeConfigError::DuplicateSignal(name) => name,
                logiksmith_core::RuntimeConfigError::UnsupportedSignalDpt { signal, .. } => signal,
                _ => return FieldError { path: "signals".to_owned(), message: error.to_string() },
            }),
            message: error.to_string(),
        },
        logiksmith_core::RuntimeConfigError::TooManySignalBindings { .. } => FieldError {
            path: "blocks.signal_bindings".to_owned(),
            message: error.to_string(),
        },
        logiksmith_core::RuntimeConfigError::UnknownSignal {
            block_id,
            endpoint,
            ..
        }
        | logiksmith_core::RuntimeConfigError::SignalDptMismatch {
            block_id,
            endpoint,
            ..
        } => FieldError {
            path: document
                .blocks
                .iter()
                .position(|block| block.id == block_id.as_str())
                .and_then(|block_index| {
                    document.blocks[block_index]
                        .signal_bindings
                        .iter()
                        .position(|binding| binding.endpoint == endpoint.as_str())
                        .map(|binding_index| {
                            format!("blocks[{block_index}].signal_bindings[{binding_index}]")
                        })
                })
                .unwrap_or_else(|| format!("{}.signal_bindings", block_path(block_id))),
            message: error.to_string(),
        },
        logiksmith_core::RuntimeConfigError::DuplicateSignalProducer { duplicate, .. } => {
            FieldError {
                path: document
                    .blocks
                    .iter()
                    .position(|block| block.id == duplicate.block_id.as_str())
                    .and_then(|block_index| {
                        document.blocks[block_index]
                            .signal_bindings
                            .iter()
                            .position(|binding| binding.endpoint == duplicate.endpoint.as_str())
                            .map(|binding_index| {
                                format!(
                                    "blocks[{block_index}].signal_bindings[{binding_index}].signal"
                                )
                            })
                    })
                    .unwrap_or_else(|| "blocks.signal_bindings".to_owned()),
                message: error.to_string(),
            }
        }
        logiksmith_core::RuntimeConfigError::SignalCycle { .. } => FieldError {
            path: "blocks.signal_bindings".to_owned(),
            message: error.to_string(),
        },
        logiksmith_core::RuntimeConfigError::InvalidBlock { block_id, .. } => FieldError {
            path: format!("{}.source", block_path(block_id)),
            message: error.to_string(),
        },
        _ => FieldError {
            path: "blocks".to_owned(),
            message: error.to_string(),
        },
    }
}

pub fn automation_revision(source: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in source {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn document_logic_revision(document: &AutomationDocument) -> u64 {
    let mut bytes = Vec::new();
    for block in &document.blocks {
        bytes.extend_from_slice(block.id.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(block.source.as_bytes());
        bytes.push(0);
    }
    automation_revision(&bytes)
}

pub fn load_automation(path: &Path) -> Result<(AutomationDocument, u16), AutomationFileError> {
    let source = fs::read(path).map_err(|source| AutomationFileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let text = String::from_utf8_lossy(&source);
    if let Ok(value) = toml::from_str::<toml::Value>(&text) {
        let legacy = ["inputs", "outputs", "knx_bindings", "logic"]
            .into_iter()
            .filter(|field| value.get(*field).is_some())
            .map(|field| FieldError {
                path: (*field).to_owned(),
                message: "legacy top-level field must move inside [[blocks]]".to_owned(),
            })
            .collect::<Vec<_>>();
        if !legacy.is_empty() {
            return Err(AutomationFileError::Invalid(legacy));
        }
    }
    let stored = toml::from_str::<StoredAutomation>(&text).map_err(AutomationFileError::Toml)?;
    build_automation(stored.document.clone()).map_err(AutomationFileError::Invalid)?;
    Ok((stored.document, stored.revision))
}

pub fn serialize_automation(
    document: &AutomationDocument,
    _revision: u16,
) -> Result<Vec<u8>, String> {
    toml::to_string_pretty(&StoredAutomation {
        revision: 0,
        document: document.clone(),
    })
    .map(|text| text.into_bytes())
    .map_err(|error| error.to_string())
}

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
    let mut automation =
        build_automation(stored.document).map_err(ConfigError::AutomationInvalid)?;
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

// ---------------------------------------------------------------------------
// Version-one NDJSON protocol
