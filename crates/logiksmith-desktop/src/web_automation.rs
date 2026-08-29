async fn get_automation(State(state): State<AppState>) -> Response {
    match load_automation(&state.store.automation_path()) {
        Ok((document, revision)) => {
            let snapshot = state.store.snapshot();
            (
                StatusCode::OK,
                Json(AutomationEnvelope {
                    document,
                    revision: u64::from(revision),
                    active_structural_revision: snapshot.logic.active_structural_revision,
                    saved_structural_revision: snapshot.logic.saved_structural_revision,
                    active_logic_revision: snapshot.logic.active_logic_revision,
                    saved_logic_revision: snapshot.logic.saved_logic_revision,
                    restart_required: snapshot.logic.restart_required,
                    blocks: block_statuses(&snapshot),
                }),
            )
                .into_response()
        }
        Err(error) => json_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct SaveAutomationRequest {
    document: AutomationDocument,
    #[serde(default, rename = "revision")]
    _revision: Option<u16>,
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
struct SaveAutomationResponse {
    #[serde(skip_serializing)]
    revision: u64,
    logic_activated: bool,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    active_logic_revision: u64,
    restart_required: bool,
    cancelled_timers: Vec<String>,
    changed_block_ids: Vec<String>,
    blocks: Vec<AutomationBlockStatus>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Serialize)]
struct FieldErrorsResponse {
    errors: Vec<FieldError>,
}

async fn put_automation(
    State(state): State<AppState>,
    Json(request): Json<SaveAutomationRequest>,
) -> Response {
    let path = state.store.automation_path();
    let candidate_structural_revision = crate::structural_revision(&request.document);
    enum SaveOutcome {
        Conflict(AutomationDocument),
        Invalid(Vec<FieldError>),
        Saved(Result<(u16, AutomationDocument), String>),
    }
    // Keep the stale check and rename under one lock. The await below happens
    // only after this guard is dropped, so the axum handler remains Send.
    let save = {
        let _guard = state
            .automation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (current, _current_revision) = match load_automation(&path) {
            Ok(value) => value,
            Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        };
        let mut document = request.document.clone();
        if merge_block_revisions(&current, &mut document).is_err() {
            SaveOutcome::Conflict(current)
        } else if let Err(errors) = build_automation(document.clone()) {
            SaveOutcome::Invalid(errors)
        } else {
            SaveOutcome::Saved(atomic_save(&path, &document).map(|revision| (revision, document)))
        }
    };
    match save {
        SaveOutcome::Conflict(current) => {
            let snapshot = state.store.snapshot();
            (
                StatusCode::CONFLICT,
                Json(AutomationEnvelope {
                    document: current,
                    revision: state.store.latest_revision(),
                    active_structural_revision: snapshot.logic.active_structural_revision,
                    saved_structural_revision: snapshot.logic.saved_structural_revision,
                    active_logic_revision: snapshot.logic.active_logic_revision,
                    saved_logic_revision: snapshot.logic.saved_logic_revision,
                    restart_required: snapshot.logic.restart_required,
                    blocks: block_statuses(&snapshot),
                }),
            )
                .into_response()
        }
        SaveOutcome::Invalid(errors) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(FieldErrorsResponse { errors }),
        )
            .into_response(),
        SaveOutcome::Saved(save_result) => match save_result {
            Ok((revision, document)) => {
                let active_structural_revision =
                    state.store.snapshot().logic.active_structural_revision;
                let mut logic_activated = false;
                let mut cancelled_timers = Vec::new();
                let mut restart_required =
                    candidate_structural_revision != active_structural_revision;
                if !restart_required {
                    if let Some(activation) = &state.activation {
                        let (reply, result) = oneshot::channel();
                        let updates = document
                            .blocks
                            .iter()
                            .map(|block| {
                                logiksmith_core::BlockActivation::new(
                                    block.id.parse().expect("validated block ID"),
                                    Some(block.source.clone()),
                                    Some(block.enabled),
                                )
                            })
                            .collect::<Vec<_>>();
                        let request = ActivationRequest {
                            updates,
                            document_revision: u64::from(revision),
                            document: document.clone(),
                            reply,
                        };
                        if activation.send(request).await.is_ok() {
                            if let Some(activation) =
                                tokio::time::timeout(Duration::from_secs(2), result)
                                    .await
                                    .ok()
                                    .and_then(|result| result.ok())
                                    .and_then(Result::ok)
                            {
                                logic_activated = true;
                                cancelled_timers = activation
                                    .result
                                    .blocks
                                    .into_iter()
                                    .flat_map(|block| {
                                        block.cancelled_timers.into_iter().map(move |timer| {
                                            format!("{}.{}", block.block_id, timer)
                                        })
                                    })
                                    .collect();
                            }
                        }
                    }
                    restart_required = !logic_activated;
                }
                state.store.set_saved_document_state(
                    u64::from(revision),
                    candidate_structural_revision,
                    restart_required,
                    &document,
                );
                let active_logic_revision = state.store.snapshot().logic.active_logic_revision;
                let snapshot = state.store.snapshot();
                (
                    StatusCode::OK,
                    Json(SaveAutomationResponse {
                        revision: u64::from(revision),
                        logic_activated,
                        active_logic_revision,
                        restart_required,
                        cancelled_timers,
                        changed_block_ids: document
                            .blocks
                            .iter()
                            .map(|block| block.id.clone())
                            .collect(),
                        blocks: block_statuses(&snapshot),
                    }),
                )
                    .into_response()
            }
            Err(error) => json_error(StatusCode::INTERNAL_SERVER_ERROR, error),
        },
    }
}

fn json_error(status: StatusCode, error: String) -> Response {
    (status, Json(ErrorResponse { error })).into_response()
}

fn atomic_save(path: &Path, document: &AutomationDocument) -> Result<u16, String> {
    let bytes = serialize_automation(document, 0)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "automation path has no file name".to_owned())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let temporary = parent.join(format!(".{file_name}.{stamp}-{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        fs::rename(&temporary, path).map_err(|error| error.to_string())?;
        Ok::<_, String>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(|_| 0)
}

fn merge_block_revisions(
    current: &AutomationDocument,
    candidate: &mut AutomationDocument,
) -> Result<(), ()> {
    for block in &mut candidate.blocks {
        if let Some(previous) = current.blocks.iter().find(|item| item.id == block.id) {
            let changed = block.enabled != previous.enabled
                || block.inputs != previous.inputs
                || block.outputs != previous.outputs
                || block.knx_bindings != previous.knx_bindings
                || block.schedules != previous.schedules
                || block.source != previous.source;
            if changed {
                if block.revision.max(1) != previous.revision.max(1) {
                    return Err(());
                }
                block.revision = previous.revision.max(1).saturating_add(1);
            } else {
                block.revision = previous.revision.max(1);
            }
        } else {
            block.revision = block.revision.max(1);
        }
    }
    Ok(())
}
