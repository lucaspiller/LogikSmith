/// Validates a complete automation document and constructs its core and
/// desktop-side KNX routing maps.
#[cfg(feature = "http-inputs")]
fn validate_http_url(path: &str, value: &str, errors: &mut Vec<FieldError>) {
    let parsed_url = match reqwest::Url::parse(value) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => Some(url),
        Ok(_) => {
            errors.push(FieldError {
                path: format!("{path}.url"),
                message: "must use http or https".to_owned(),
            });
            None
        }
        Err(error) => {
            errors.push(FieldError {
                path: format!("{path}.url"),
                message: format!("must be a valid HTTP URL: {error}"),
            });
            None
        }
    };
    if let Some(url) = parsed_url
        && (url.username() != "" || url.password().is_some() || url.fragment().is_some())
    {
        errors.push(FieldError {
            path: format!("{path}.url"),
            message: "must not include user-info or a fragment".to_owned(),
        });
    }
}

pub fn build_automation(
    document: AutomationDocument,
) -> Result<AutomationRuntime, Vec<FieldError>> {
    build_automation_with_limits(document, logiksmith_core::RuntimeLimits::desktop())
}

/// Builds an automation using the host-selected semantic resource profile.
/// The compatibility wrapper above retains the desktop defaults for callers
/// which do not have a host profile yet.
pub fn build_automation_with_limits(
    document: AutomationDocument,
    limits: logiksmith_core::RuntimeLimits,
) -> Result<AutomationRuntime, Vec<FieldError>> {
    let mut errors = Vec::new();
    // A stripped host must reject external-source configuration before it can
    // construct any source runtime or routing map. This is intentionally a
    // validation error (rather than silently dropping the source), so a
    // configuration cannot appear healthy on a binary which lacks its code.
    if !cfg!(feature = "http-inputs")
        && (!document.http_polls.is_empty()
            || document
                .blocks
                .iter()
                .any(|block| !block.http_bindings.is_empty()))
    {
        errors.push(FieldError {
            path: "http_polls".to_owned(),
            message: "feature_disabled: HTTP inputs are unavailable in this build; enable the `http-inputs` Cargo feature".to_owned(),
        });
    }
    if !cfg!(feature = "webhook-inputs")
        && (!document.webhook_inputs.is_empty()
            || document
                .blocks
                .iter()
                .any(|block| !block.webhook_bindings.is_empty()))
    {
        errors.push(FieldError {
            path: "webhook_inputs".to_owned(),
            message: "feature_disabled: webhook inputs are unavailable in this build; enable the `webhook-inputs` Cargo feature".to_owned(),
        });
    }
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
        let name = match SignalName::parse(&signal.name) {
            Ok(name) => name,
            Err(error) => {
                errors.push(FieldError {
                    path: format!("{path}.name"),
                    message: error.to_string(),
                });
                continue;
            }
        };
        if name.as_str() != signal.name {
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
        if signal_dpts.insert(name.clone(), dpt).is_some() {
            errors.push(FieldError {
                path: format!("{path}.name"),
                message: "must be unique".to_owned(),
            });
            continue;
        }
        signals.push(SignalRuntime { name, dpt });
    }
    let mut source_dpts = HashMap::<String, Dpt>::new();
    let mut http_polls = Vec::new();
    let mut poll_names = HashSet::new();
    let mut source_values_and_bindings = 0usize;
    if document.http_polls.len() + document.webhook_inputs.len() > MAX_EXTERNAL_SOURCES {
        errors.push(FieldError {
            path: "http_polls".to_owned(),
            message: format!(
                "http_polls plus webhook_inputs must contain at most {MAX_EXTERNAL_SOURCES} sources"
            ),
        });
    }
    for (poll_index, poll) in document.http_polls.iter().enumerate() {
        let path = format!("http_polls[{poll_index}]");
        let Some(name) = source_name(&format!("{path}.name"), &poll.name, &mut errors) else {
            continue;
        };
        if !poll_names.insert(name.clone()) {
            errors.push(FieldError {
                path: format!("{path}.name"),
                message: "must be unique".to_owned(),
            });
        }
        if poll.url.len() > MAX_HTTP_URL_BYTES {
            errors.push(FieldError {
                path: format!("{path}.url"),
                message: format!("must not exceed {MAX_HTTP_URL_BYTES} bytes"),
            });
        }
        #[cfg(feature = "http-inputs")]
        validate_http_url(&path, &poll.url, &mut errors);
        let every = parse_source_duration(&format!("{path}.every"), &poll.every, &mut errors);
        if let Some(every) = every
            && !(std::time::Duration::from_secs(MIN_POLL_INTERVAL_SECONDS)
                ..=std::time::Duration::from_secs(MAX_POLL_INTERVAL_SECONDS))
                .contains(&every)
        {
            errors.push(FieldError {
                path: format!("{path}.every"),
                message: "must be between 1s and 24h".to_owned(),
            });
        }
        let timeout =
            parse_timeout_duration(&format!("{path}.timeout"), &poll.timeout, &mut errors);
        if let (Some(timeout), Some(every)) = (timeout, every)
            && timeout > every
        {
            errors.push(FieldError {
                path: format!("{path}.timeout"),
                message: "must not exceed every".to_owned(),
            });
        }
        let stale_after = parse_source_duration(
            &format!("{path}.stale_after"),
            &poll.stale_after,
            &mut errors,
        );
        if let (Some(stale_after), Some(every)) = (stale_after, every)
            && (stale_after < every
                || stale_after > std::time::Duration::from_secs(MAX_FRESHNESS_SECONDS))
        {
            errors.push(FieldError {
                path: format!("{path}.stale_after"),
                message: "must be at least every and no more than 7d".to_owned(),
            });
        }
        if poll.headers.len() > MAX_HTTP_HEADERS {
            errors.push(FieldError {
                path: format!("{path}.headers"),
                message: format!("must contain at most {MAX_HTTP_HEADERS} headers"),
            });
        }
        let mut headers = Vec::new();
        let mut header_names = HashSet::new();
        for (header_index, header) in poll.headers.iter().enumerate() {
            let header_path = format!("{path}.headers[{header_index}]");
            if !header_names.insert(header.name.to_ascii_lowercase()) {
                errors.push(FieldError {
                    path: format!("{header_path}.name"),
                    message: "must be unique within a poll".to_owned(),
                });
            }
            if let Some(value) = validate_header(&header_path, header, &mut errors) {
                headers.push(value);
            }
        }
        if poll.values.is_empty() {
            errors.push(FieldError {
                path: format!("{path}.values"),
                message: "must contain at least one extracted value".to_owned(),
            });
        }
        let mut values = Vec::new();
        let mut value_names = HashSet::new();
        for (value_index, value) in poll.values.iter().enumerate() {
            source_values_and_bindings = source_values_and_bindings.saturating_add(1);
            let value_path = format!("{path}.values[{value_index}]");
            let Some(name) = source_name(&format!("{value_path}.name"), &value.name, &mut errors)
            else {
                continue;
            };
            if !value_names.insert(name.clone()) {
                errors.push(FieldError {
                    path: format!("{value_path}.name"),
                    message: "must be unique within a poll".to_owned(),
                });
                continue;
            }
            let dpt = match parse_dpt(&format!("{value_path}.dpt"), &value.dpt) {
                Ok(dpt) => dpt,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            validate_json_pointer(
                &format!("{value_path}.json_pointer"),
                &value.json_pointer,
                &mut errors,
            );
            if let Some(previous) = source_dpts.insert(name.clone(), dpt) {
                errors.push(FieldError {
                    path: format!("{value_path}.name"),
                    message: format!("must be unique; conflicts with DPT {previous}"),
                });
                continue;
            }
            values.push(HttpPollValueRuntime {
                name,
                dpt,
                json_pointer: value.json_pointer.clone(),
            });
        }
        if let (Some(every), Some(timeout), Some(stale_after)) = (every, timeout, stale_after) {
            http_polls.push(HttpPollRuntime {
                name,
                url: poll.url.clone(),
                every,
                timeout,
                stale_after,
                headers,
                values,
            });
        }
    }
    let mut webhook_inputs = Vec::new();
    for (source_index, source) in document.webhook_inputs.iter().enumerate() {
        let path = format!("webhook_inputs[{source_index}]");
        let Some(name) = source_name(&format!("{path}.name"), &source.name, &mut errors) else {
            continue;
        };
        let dpt = match parse_dpt(&format!("{path}.dpt"), &source.dpt) {
            Ok(dpt) => dpt,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        validate_json_pointer(
            &format!("{path}.json_pointer"),
            &source.json_pointer,
            &mut errors,
        );
        let bearer_token = match &source.bearer_token_env {
            None => None,
            Some(variable) if variable.is_empty() => {
                errors.push(FieldError {
                    path: format!("{path}.bearer_token_env"),
                    message: "must not be empty".to_owned(),
                });
                None
            }
            Some(variable) => match std::env::var(variable) {
                Ok(value) if !value.is_empty() && !value.contains(['\r', '\n']) => Some(value),
                Ok(_) => {
                    errors.push(FieldError {
                        path: format!("{path}.bearer_token_env"),
                        message:
                            "environment variable must contain a non-empty token without CR/LF"
                                .to_owned(),
                    });
                    None
                }
                Err(_) => {
                    errors.push(FieldError {
                        path: format!("{path}.bearer_token_env"),
                        message: format!("environment variable {variable:?} is not set"),
                    });
                    None
                }
            },
        };
        source_values_and_bindings = source_values_and_bindings.saturating_add(1);
        if let Some(previous) = source_dpts.insert(name.clone(), dpt) {
            errors.push(FieldError {
                path: format!("{path}.name"),
                message: format!("must be unique; conflicts with DPT {previous}"),
            });
            continue;
        }
        webhook_inputs.push(WebhookInputRuntime {
            name,
            dpt,
            json_pointer: source.json_pointer.clone(),
            bearer_token,
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
    let mut signal_to_inputs: HashMap<SignalName, Vec<BlockSignalInputBinding>> = HashMap::new();
    let mut output_to_signal = HashMap::new();
    let mut http_to_inputs: HashMap<String, Vec<BlockExternalInputBinding>> = HashMap::new();
    let mut webhook_to_inputs: HashMap<String, Vec<BlockExternalInputBinding>> = HashMap::new();
    let mut external_binding_count = 0usize;
    let mut address_dpts = HashMap::new();
    let mut address_origins: HashMap<GroupAddress, (BlockId, EndpointName)> = HashMap::new();
    let mut signal_producers: HashMap<SignalName, (BlockId, EndpointName)> = HashMap::new();
    let mut signal_binding_count = 0usize;
    for (block_index, block) in document.blocks.iter().enumerate() {
        let block_path = format!("blocks[{block_index}]");
        let id = match block.id.parse::<BlockId>() {
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
        let mut endpoint_to_external = HashMap::new();
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
                let previous_origin = address_origins
                    .get(&address)
                    .map(|(block, endpoint)| format!("{block}.{endpoint}"))
                    .unwrap_or_else(|| "another block.unknown".to_owned());
                errors.push(FieldError {
                    path: format!("{path}.group_address"),
                    message: format!("DPT conflicts with {previous_origin} at {address}"),
                });
                continue;
            }
            address_dpts.insert(address, dpt);
            address_origins.insert(address, (id.clone(), name.clone()));
            endpoint_to_address.insert(name.clone(), address);
            match endpoint_directions.get(&name) {
                Some(EndpointDirection::Input) => address_to_inputs
                    .entry(address)
                    .or_default()
                    .push(BlockInputBinding {
                        block_id: id.clone(),
                        endpoint: name,
                        dpt,
                        address,
                    }),
                Some(EndpointDirection::Output) => {
                    output_to_address.insert((id.clone(), name), address);
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
            let signal = match binding.signal.parse::<SignalName>() {
                Ok(signal) => signal,
                Err(_) => {
                    errors.push(FieldError {
                        path: format!("{path}.signal"),
                        message: "must reference an existing signal".to_owned(),
                    });
                    continue;
                }
            };
            let Some(&signal_dpt) = signal_dpts.get(&signal) else {
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
            endpoint_to_signal.insert(name.clone(), signal.clone());
            if endpoint_directions.get(&name) == Some(&EndpointDirection::Input) {
                signal_to_inputs
                    .entry(signal.clone())
                    .or_default()
                    .push(BlockSignalInputBinding {
                        block_id: id.clone(),
                        endpoint: name,
                        dpt,
                        signal,
                    });
            } else if endpoint_directions.get(&name) == Some(&EndpointDirection::Output) {
                if let Some((previous_block, previous_endpoint)) =
                    signal_producers.insert(signal.clone(), (id.clone(), name.clone()))
                {
                    errors.push(FieldError {
                        path: format!("{path}.signal"),
                        message: format!(
                            "must have at most one producer; already produced by {previous_block}.{previous_endpoint}"
                        ),
                    });
                }
                output_to_signal.insert((id.clone(), name), signal);
            }
        }
        for (index, binding) in block.http_bindings.iter().enumerate() {
            external_binding_count = external_binding_count.saturating_add(1);
            let path = format!("{block_path}.http_bindings[{index}]");
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
            if endpoint_directions.get(&name) != Some(&EndpointDirection::Input) {
                errors.push(FieldError {
                    path: format!("{path}.endpoint"),
                    message: "must reference an input endpoint".to_owned(),
                });
                continue;
            }
            let Some(&source_dpt) = source_dpts.get(&binding.source) else {
                errors.push(FieldError {
                    path: format!("{path}.source"),
                    message: "must reference an HTTP poll value".to_owned(),
                });
                continue;
            };
            if source_dpt != dpt {
                errors.push(FieldError {
                    path: format!("{path}.source"),
                    message: format!("DPT {source_dpt} conflicts with endpoint {name} DPT {dpt}"),
                });
                continue;
            }
            if !http_polls
                .iter()
                .any(|poll| poll.values.iter().any(|value| value.name == binding.source))
            {
                errors.push(FieldError {
                    path: format!("{path}.source"),
                    message: "must reference an HTTP poll value, not a webhook".to_owned(),
                });
                continue;
            }
            if endpoint_to_address.contains_key(&name)
                || endpoint_to_signal.contains_key(&name)
                || endpoint_to_external.contains_key(&name)
            {
                errors.push(FieldError {
                    path: format!("{path}.endpoint"),
                    message:
                        "must have exactly one binding across KNX, signals, HTTP, and webhooks"
                            .to_owned(),
                });
                continue;
            }
            endpoint_to_external.insert(name.clone(), binding.source.clone());
            http_to_inputs
                .entry(binding.source.clone())
                .or_default()
                .push(BlockExternalInputBinding {
                    block_id: id.clone(),
                    endpoint: name,
                    dpt,
                    source: binding.source.clone(),
                });
        }
        for (index, binding) in block.webhook_bindings.iter().enumerate() {
            external_binding_count = external_binding_count.saturating_add(1);
            let path = format!("{block_path}.webhook_bindings[{index}]");
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
            if endpoint_directions.get(&name) != Some(&EndpointDirection::Input) {
                errors.push(FieldError {
                    path: format!("{path}.endpoint"),
                    message: "must reference an input endpoint".to_owned(),
                });
                continue;
            }
            let Some(&source_dpt) = source_dpts.get(&binding.source) else {
                errors.push(FieldError {
                    path: format!("{path}.source"),
                    message: "must reference a webhook input".to_owned(),
                });
                continue;
            };
            if source_dpt != dpt {
                errors.push(FieldError {
                    path: format!("{path}.source"),
                    message: format!("DPT {source_dpt} conflicts with endpoint {name} DPT {dpt}"),
                });
                continue;
            }
            if !webhook_inputs
                .iter()
                .any(|source| source.name == binding.source)
            {
                errors.push(FieldError {
                    path: format!("{path}.source"),
                    message: "must reference a webhook input, not an HTTP poll value".to_owned(),
                });
                continue;
            }
            if endpoint_to_address.contains_key(&name)
                || endpoint_to_signal.contains_key(&name)
                || endpoint_to_external.contains_key(&name)
            {
                errors.push(FieldError {
                    path: format!("{path}.endpoint"),
                    message:
                        "must have exactly one binding across KNX, signals, HTTP, and webhooks"
                            .to_owned(),
                });
                continue;
            }
            endpoint_to_external.insert(name.clone(), binding.source.clone());
            webhook_to_inputs
                .entry(binding.source.clone())
                .or_default()
                .push(BlockExternalInputBinding {
                    block_id: id.clone(),
                    endpoint: name,
                    dpt,
                    source: binding.source.clone(),
                });
        }
        for endpoint in &endpoints {
            if !endpoint_to_address.contains_key(&endpoint.name)
                && !endpoint_to_signal.contains_key(&endpoint.name)
                && !endpoint_to_external.contains_key(&endpoint.name)
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
                        "endpoint {} must have exactly one binding across KNX, signals, HTTP, and webhooks",
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
            id,
            revision: block.revision.max(1),
            enabled: block.enabled,
            engine_config,
            endpoint_to_address,
            endpoint_to_signal,
            endpoint_to_external,
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
    if source_values_and_bindings.saturating_add(external_binding_count)
        > MAX_EXTERNAL_VALUES_AND_BINDINGS
    {
        errors.push(FieldError {
            path: "blocks".to_owned(),
            message: format!("extracted values plus bindings must contain at most {MAX_EXTERNAL_VALUES_AND_BINDINGS} entries"),
        });
    }
    for source in source_dpts.keys() {
        if !http_to_inputs.contains_key(source) && !webhook_to_inputs.contains_key(source) {
            errors.push(FieldError {
                path: "http_polls".to_owned(),
                message: format!("source {source:?} must have at least one block binding"),
            });
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let core_blocks = blocks
        .iter()
        .map(|block| {
            let mut config = CoreBlockConfig::with_schedules(
                block.id.clone(),
                block.enabled,
                block.engine_config.endpoints.clone(),
                block.engine_config.logic.source.clone(),
                block.schedules.clone(),
            );
            config.signal_bindings = document
                .blocks
                .iter()
                .find(|candidate| candidate.id == block.id.to_string())
                .map(|candidate| {
                    candidate
                        .signal_bindings
                        .iter()
                        .map(|binding| {
                            let endpoint = block
                                .engine_config
                                .endpoints
                                .iter()
                                .find(|endpoint| endpoint.name.as_str() == binding.endpoint)
                                .map(|endpoint| endpoint.name.clone())
                                .expect("validated endpoint");
                            CoreSignalBinding::new(
                                endpoint.clone(),
                                block
                                    .endpoint_to_signal
                                    .get(&endpoint)
                                    .cloned()
                                    .expect("validated signal binding"),
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
        .map(|signal| CoreSignalConfig::new(signal.name.clone(), signal.dpt))
        .collect();
    let core_config = CoreRuntimeConfig::with_signals(core_blocks, core_signals);
    if let Err(error) = core_config.validate_with_limits(&limits) {
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
        http_to_inputs,
        webhook_to_inputs,
        http_polls,
        webhook_inputs,
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
                _ => {
                    return FieldError {
                        path: "signals".to_owned(),
                        message: error.to_string(),
                    };
                }
            }),
            message: error.to_string(),
        },
        logiksmith_core::RuntimeConfigError::TooManySignalBindings { .. } => FieldError {
            path: "blocks.signal_bindings".to_owned(),
            message: error.to_string(),
        },
        logiksmith_core::RuntimeConfigError::UnknownSignal {
            block_id, endpoint, ..
        }
        | logiksmith_core::RuntimeConfigError::SignalDptMismatch {
            block_id, endpoint, ..
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
include!("automation_file.rs");

// ---------------------------------------------------------------------------
// Version-one NDJSON protocol
