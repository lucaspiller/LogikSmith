fn configure_message(config: &RuntimeConfig) -> Message {
    let mut group_addresses: Vec<_> = config
        .automation
        .address_dpts
        .iter()
        .map(|(address, dpt)| GroupAddressDpt {
            address: address.to_string(),
            dpt: DptMessage::from_core(*dpt),
        })
        .collect();
    group_addresses.sort_by_key(|entry| {
        entry
            .address
            .parse::<GroupAddress>()
            .map(|address| (address.main, address.middle, address.subgroup))
            .unwrap_or((u8::MAX, u8::MAX, u8::MAX))
    });
    Message::Configure(Configure {
        v: PROTOCOL_VERSION,
        message_type: "configure".to_owned(),
        connection: TunnelingConnection {
            connection_type: "tunneling".to_owned(),
            gateway_ip: config.connection.gateway_ip.to_string(),
            gateway_port: config.connection.gateway_port,
            local_ip: config.connection.local_ip.map(|ip| ip.to_string()),
        },
        group_addresses,
    })
}

fn shutdown_message() -> Message {
    Message::Shutdown(Shutdown {
        v: PROTOCOL_VERSION,
        message_type: "shutdown".to_owned(),
    })
}

async fn dispatch_effects(
    store: &DiagnosticStore,
    stdin: &mut ChildStdin,
    automation: &AutomationRuntime,
    block_id: &BlockId,
    effects: Vec<OutputEffect>,
    next_request_id: &mut u64,
    pending: &mut HashSet<u64>,
) -> Result<(), HostError> {
    for effect in effects {
        let endpoint = effect.endpoint;
        let value = effect.value;
        let Some(destination) = automation
            .output_to_address
            .get(&(block_id.clone(), endpoint.clone()))
            .copied()
        else {
            tracing::error!(target: "logiksmith", block = %block_id, endpoint = %endpoint, "core returned an unresolved output effect");
            continue;
        };
        let request_id = *next_request_id;
        *next_request_id = next_request_id.checked_add(1).ok_or_else(|| {
            HostError::Protocol(ProtocolError::Field("request_id", "exhausted".to_owned()))
        })?;
        let dpt = automation
            .block(block_id)
            .and_then(|block| block.endpoint_dpts.get(&endpoint))
            .copied()
            .ok_or_else(|| {
                HostError::Protocol(ProtocolError::Field(
                    "output",
                    "missing endpoint DPT".to_owned(),
                ))
            })?;
        if value.dpt() != dpt {
            tracing::error!(target: "logiksmith", endpoint = %endpoint, "core returned an output value with the wrong DPT");
            continue;
        }
        pending.insert(request_id);
        store.record_write_requested(
            request_id,
            block_id,
            endpoint.clone(),
            destination,
            dpt,
            value,
        );
        let message = Message::KnxWrite(KnxWrite {
            v: PROTOCOL_VERSION,
            message_type: "knx_write".to_owned(),
            request_id,
            destination: destination.to_string(),
            dpt: DptMessage::from_core(dpt),
            value: ValueMessage::from_core(value),
        });
        if let Err(error) = send_message(stdin, &message).await {
            pending.remove(&request_id);
            store.record_write_result(request_id, false, Some(error.to_string()));
            return Err(error);
        }
    }
    Ok(())
}

async fn read_message(reader: &mut BufReader<ChildStdout>) -> Result<Message, HostError> {
    let mut line = String::new();
    let bytes = reader.read_line(&mut line).await?;
    if bytes == 0 {
        return Err(HostError::StdoutEof);
    }
    Ok(parse_message(line.trim_end_matches(['\r', '\n']))?)
}

async fn send_message(stdin: &mut ChildStdin, message: &Message) -> Result<(), HostError> {
    stdin.write_all(encode_message(message)?.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;
    Ok(())
}

fn init_logging(config: LoggingConfig, store: DiagnosticStore) {
    diagnostics::activate_tracing_store(store);
    let filter = logging_filter(config);
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_filter(filter.clone());
    let diagnostics_layer = diagnostics::tracing_layer().with_filter(filter);
    let _ = tracing_subscriber::registry()
        .with(fmt_layer)
        .with(diagnostics_layer)
        .try_init();
}

fn logging_filter(config: LoggingConfig) -> EnvFilter {
    EnvFilter::builder()
        .with_default_directive(config.level.into())
        .parse_lossy(format!(
            "logiksmith={},bridge.xknx={}",
            config.level, config.bridge_level
        ))
}

async fn terminate_child(child: &mut Child) {
    if time::timeout(Duration::from_millis(250), child.wait())
        .await
        .is_ok()
    {
        return;
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn format_status(status: std::process::ExitStatus) -> String {
    status
        .code()
        .map_or_else(|| "signal".to_owned(), |code| code.to_string())
}

impl fmt::Display for DptMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{:03}", self.major, self.subtype)
    }
}
