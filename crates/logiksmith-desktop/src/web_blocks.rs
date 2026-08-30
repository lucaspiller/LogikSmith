// Block-scoped authoring and debugging routes. This file is included into the
// `web` module, so it deliberately uses ordinary comments instead of inner
// module documentation.

use serde::de::DeserializeOwned;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockSourceRequest {
    source: String,
    #[serde(default, alias = "sourceFingerprint")]
    source_fingerprint: Option<String>,
    #[serde(alias = "expectedRevision", alias = "expectedActiveRevision")]
    #[serde(deserialize_with = "crate::wire_revision::deserialize")]
    expected_revision: u64,
    #[serde(
        default,
        alias = "expectedStructuralRevision",
        alias = "expectedActiveStructuralRevision"
    )]
    #[serde(deserialize_with = "crate::wire_revision::deserialize_option")]
    expected_structural_revision: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockEnabledRequest {
    enabled: bool,
    #[serde(alias = "expectedRevision", alias = "expectedActiveRevision")]
    #[serde(deserialize_with = "crate::wire_revision::deserialize")]
    expected_revision: u64,
    #[serde(
        default,
        alias = "expectedStructuralRevision",
        alias = "expectedActiveStructuralRevision"
    )]
    #[serde(deserialize_with = "crate::wire_revision::deserialize_option")]
    expected_structural_revision: Option<u64>,
}

#[derive(Debug, Serialize)]
struct BlockValidationResponse {
    status: &'static str,
    block_id: String,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    block_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    structural_revision: u64,
    source_fingerprint: String,
    errors: Vec<SourceDiagnostic>,
}

#[derive(Debug, Serialize)]
struct SourceDiagnostic {
    category: String,
    message: String,
    line: Option<usize>,
}

#[derive(Debug, Serialize)]
struct BlockConflictResponse {
    error: String,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    current_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    current_structural_revision: u64,
}

#[derive(Debug, Serialize)]
struct BlockMutationResponse {
    block_id: String,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    active_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    saved_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    active_logic_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    saved_logic_revision: u64,
    active_enabled: bool,
    saved_enabled: bool,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    structural_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    active_structural_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    saved_structural_revision: u64,
    restart_required: bool,
    cancelled_timers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

fn malformed_block_json<T: DeserializeOwned>(body: Bytes) -> Result<T, Response> {
    serde_json::from_slice(&body).map_err(|error| {
        let status = if error.is_syntax() || error.is_eof() {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::UNPROCESSABLE_ENTITY
        };
        (
            status,
            Json(FieldErrorsResponse {
                errors: vec![FieldError {
                    path: "request".to_owned(),
                    message: error.to_string(),
                }],
            }),
        )
            .into_response()
    })
}

fn source_diagnostics(error: &logiksmith_core::LogicError) -> Vec<SourceDiagnostic> {
    vec![SourceDiagnostic {
        category: error.category().to_owned(),
        message: error.message().to_owned(),
        line: error.line(),
    }]
}

fn source_field_errors(error: &logiksmith_core::LogicError) -> Vec<FieldError> {
    vec![FieldError {
        path: "source".to_owned(),
        message: error.to_string(),
    }]
}

fn active_block<'a>(snapshot: &'a Snapshot, block_id: &str) -> Option<&'a BlockSnapshot> {
    snapshot.blocks.iter().find(|block| block.id == block_id)
}

fn block_conflict(snapshot: &Snapshot, block_id: &str, message: &'static str) -> Response {
    let block = active_block(snapshot, block_id);
    (
        StatusCode::CONFLICT,
        Json(BlockConflictResponse {
            error: message.to_owned(),
            current_revision: block.map(|block| block.active_revision).unwrap_or(1),
            current_structural_revision: snapshot.logic.active_structural_revision,
        }),
    )
        .into_response()
}

fn check_block_cas(
    snapshot: &Snapshot,
    block_id: &str,
    expected_revision: u64,
    expected_structural_revision: Option<u64>,
) -> Result<(), Response> {
    let Some(block) = active_block(snapshot, block_id) else {
        return Err(json_error(StatusCode::NOT_FOUND, "unknown logic block".to_owned()));
    };
    if block.active_revision != expected_revision
        || expected_structural_revision != Some(snapshot.logic.active_structural_revision)
        || snapshot.logic.restart_required
    {
        return Err(block_conflict(
            snapshot,
            block_id,
            "active block or structural revision changed; refresh the dashboard and retry",
        ));
    }
    Ok(())
}

async fn validate_block(
    AxumPath(block_id): AxumPath<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let request = match malformed_block_json::<BlockSourceRequest>(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    // The fingerprint is a browser correlation hint, not an authorization or
    // CAS token. The server always computes the authoritative value below.
    let _client_fingerprint = request.source_fingerprint.as_deref();
    let snapshot = state.store.snapshot();
    if let Err(response) = check_block_cas(
        &snapshot,
        &block_id,
        request.expected_revision,
        request.expected_structural_revision,
    ) {
        return response;
    }
    let fingerprint = crate::source_fingerprint(&request.source);
    let response = match logiksmith_core::Runtime::validate_source(&request.source) {
        Ok(_) => BlockValidationResponse {
            status: "valid",
            block_id,
            block_revision: request.expected_revision,
            structural_revision: snapshot.logic.active_structural_revision,
            source_fingerprint: fingerprint,
            errors: Vec::new(),
        },
        Err(error) => BlockValidationResponse {
            status: "invalid",
            block_id,
            block_revision: request.expected_revision,
            structural_revision: snapshot.logic.active_structural_revision,
            source_fingerprint: fingerprint,
            errors: source_diagnostics(&error),
        },
    };
    (StatusCode::OK, Json(response)).into_response()
}

struct PreparedMutation {
    previous: AutomationDocument,
    candidate: AutomationDocument,
    document_revision: u64,
    changed: bool,
}

enum PrepareMutation {
    Ready(PreparedMutation),
    Response(Response),
}

fn prepare_mutation(
    state: &AppState,
    block_id: &str,
    expected_revision: u64,
    expected_structural_revision: Option<u64>,
    update: impl FnOnce(&mut AutomationBlock),
    source_for_validation: Option<&str>,
) -> PrepareMutation {
    let path = state.store.automation_path();
    let _guard = state
        .automation_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let snapshot = state.store.snapshot();
    if let Err(response) = check_block_cas(
        &snapshot,
        block_id,
        expected_revision,
        expected_structural_revision,
    ) {
        return PrepareMutation::Response(response);
    }
    let (previous, _file_revision) = match load_automation(&path) {
        Ok(value) => value,
        Err(error) => {
            return PrepareMutation::Response(json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                error.to_string(),
            ));
        }
    };
    let Some(previous_block) = previous.blocks.iter().find(|block| block.id == block_id) else {
        return PrepareMutation::Response(json_error(
            StatusCode::NOT_FOUND,
            "unknown logic block".to_owned(),
        ));
    };
    if previous_block.revision.max(1) != expected_revision
        || structural_revision(&previous) != snapshot.logic.active_structural_revision
    {
        return PrepareMutation::Response(block_conflict(
            &snapshot,
            block_id,
            "active block or structural revision changed; refresh the dashboard and retry",
        ));
    }
    if let Some(source) = source_for_validation {
        if let Err(error) = logiksmith_core::Runtime::validate_source(source) {
            return PrepareMutation::Response((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(FieldErrorsResponse {
                    errors: source_field_errors(&error),
                }),
            )
                .into_response());
        }
    }
    let mut candidate = previous.clone();
    let block = candidate
        .blocks
        .iter_mut()
        .find(|block| block.id == block_id)
        .expect("validated block exists in candidate document");
    let before = block.clone();
    update(block);
    let changed = *block != before;
    if changed {
        block.revision = before.revision.max(1).saturating_add(1);
    } else {
        block.revision = before.revision.max(1);
    }
    if let Err(errors) = build_automation(candidate.clone()) {
        return PrepareMutation::Response((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(FieldErrorsResponse { errors }),
        )
            .into_response());
    }
    if changed {
        if let Err(error) = atomic_save(&path, &candidate) {
            return PrepareMutation::Response(json_error(StatusCode::INTERNAL_SERVER_ERROR, error));
        }
    }
    PrepareMutation::Ready(PreparedMutation {
        previous,
        candidate,
        document_revision: snapshot
            .saved_automation_revision
            .saturating_add(u64::from(changed)),
        changed,
    })
}

async fn finish_mutation(
    state: AppState,
    block_id: String,
    prepared: PreparedMutation,
    update: logiksmith_core::BlockActivation,
    source_fingerprint: Option<String>,
) -> Response {
    let Some(activation) = state.activation.clone() else {
        // `prepare_mutation` writes before handing the candidate to the
        // serialized runtime owner. Without an owner there is no safe way to
        // make the new document active, so restore the previous bytes.
        if prepared.changed {
            let _ = restore_document(&state.store.automation_path(), &prepared.previous);
        }
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime activation service is unavailable".to_owned(),
        );
    };
    if !prepared.changed {
        return mutation_response(
            &state.store.snapshot(),
            &block_id,
            Vec::new(),
            source_fingerprint,
            None,
        );
    }
    let (reply, result) = tokio::sync::oneshot::channel();
    if activation
        .send(ActivationRequest {
            updates: vec![update],
            document_revision: prepared.document_revision,
            document: prepared.candidate.clone(),
            reply,
        })
        .await
        .is_err()
    {
        let _ = restore_document(&state.store.automation_path(), &prepared.previous);
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime activation service is unavailable".to_owned(),
        );
    }
    let activation = match result.await {
        Ok(Ok(activation)) => activation,
        Ok(Err(error)) => {
            let restored = restore_document(&state.store.automation_path(), &prepared.previous);
            if let Err(restore_error) = restored {
                return json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("activation failed ({error}); rollback failed ({restore_error})"),
                );
            }
            return json_error(StatusCode::SERVICE_UNAVAILABLE, error);
        }
        Err(_) => {
            let _ = restore_document(&state.store.automation_path(), &prepared.previous);
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "runtime activation service is unavailable".to_owned(),
            );
        }
    };
    state.store.set_saved_document_state(
        activation.document_revision,
        state.store.snapshot().logic.active_structural_revision,
        false,
        &prepared.candidate,
    );
    let cancelled_timers = activation
        .result
        .blocks
        .into_iter()
        .find(|block| block.block_id.to_string() == block_id)
        .map(|block| {
            block
                .cancelled_timers
                .into_iter()
                .map(|timer| timer.to_string())
                .collect()
        })
        .unwrap_or_default();
    mutation_response(
        &state.store.snapshot(),
        &block_id,
        cancelled_timers,
        source_fingerprint,
        None,
    )
}

fn mutation_response(
    snapshot: &Snapshot,
    block_id: &str,
    cancelled_timers: Vec<String>,
    source_fingerprint: Option<String>,
    message: Option<String>,
) -> Response {
    let Some(block) = active_block(snapshot, block_id) else {
        return json_error(StatusCode::NOT_FOUND, "unknown logic block".to_owned());
    };
    (
        StatusCode::OK,
        Json(BlockMutationResponse {
            block_id: block_id.to_owned(),
            active_revision: block.active_revision,
            saved_revision: block.saved_revision,
            active_logic_revision: block.active_logic_revision,
            saved_logic_revision: block.saved_logic_revision,
            active_enabled: block.active_enabled,
            saved_enabled: block.saved_enabled,
            structural_revision: snapshot.logic.active_structural_revision,
            active_structural_revision: snapshot.logic.active_structural_revision,
            saved_structural_revision: snapshot.logic.saved_structural_revision,
            restart_required: snapshot.logic.restart_required,
            cancelled_timers,
            source_fingerprint,
            message,
        }),
    )
        .into_response()
}

fn restore_document(path: &Path, document: &AutomationDocument) -> Result<(), String> {
    atomic_save(path, document).map(|_| ())
}

async fn activate_block_source(
    AxumPath(block_id): AxumPath<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let request = match malformed_block_json::<BlockSourceRequest>(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let _client_fingerprint = request.source_fingerprint.as_deref();
    let fingerprint = crate::source_fingerprint(&request.source);
    let prepared = prepare_mutation(
        &state,
        &block_id,
        request.expected_revision,
        request.expected_structural_revision,
        |block| block.source = request.source.clone(),
        Some(&request.source),
    );
    let prepared = match prepared {
        PrepareMutation::Ready(prepared) => prepared,
        PrepareMutation::Response(response) => return response,
    };
    let source = prepared
        .candidate
        .blocks
        .iter()
        .find(|block| block.id == block_id)
        .map(|block| block.source.clone())
        .expect("validated block exists");
    finish_mutation(
        state,
        block_id.clone(),
        prepared,
        logiksmith_core::BlockActivation::source(
            block_id.parse().expect("validated block ID"),
            source,
        ),
        Some(fingerprint),
    )
    .await
}

async fn set_block_enabled(
    AxumPath(block_id): AxumPath<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let request = match malformed_block_json::<BlockEnabledRequest>(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let enabled = request.enabled;
    let prepared = prepare_mutation(
        &state,
        &block_id,
        request.expected_revision,
        request.expected_structural_revision,
        move |block| block.enabled = enabled,
        None,
    );
    let prepared = match prepared {
        PrepareMutation::Ready(prepared) => prepared,
        PrepareMutation::Response(response) => return response,
    };
    finish_mutation(
        state,
        block_id.clone(),
        prepared,
        logiksmith_core::BlockActivation::enabled(
            block_id.parse().expect("validated block ID"),
            enabled,
        ),
        None,
    )
    .await
}

async fn simulate_block(
    AxumPath(block_id): AxumPath<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let mut payload = match malformed_block_json::<SimulationPayload>(body) {
        Ok(payload) => payload,
        Err(response) => return response,
    };
    if payload.block_id != block_id {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(FieldErrorsResponse {
                errors: vec![FieldError {
                    path: "block_id".to_owned(),
                    message: "must match the URL block ID".to_owned(),
                }],
            }),
        )
            .into_response();
    }
    if payload.source.is_none() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(FieldErrorsResponse {
                errors: vec![FieldError {
                    path: "source".to_owned(),
                    message: "is required for block-scoped simulation".to_owned(),
                }],
            }),
        )
            .into_response();
    }
    if payload.expected_structural_revision.is_none() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(FieldErrorsResponse {
                errors: vec![FieldError {
                    path: "expected_structural_revision".to_owned(),
                    message: "is required for block-scoped simulation".to_owned(),
                }],
            }),
        )
            .into_response();
    }
    let snapshot = state.store.snapshot();
    if let Err(response) = check_block_cas(
        &snapshot,
        &block_id,
        payload.expected_logic_revision,
        payload.expected_structural_revision,
    ) {
        return response;
    }
    // Ensure the source fingerprint is returned from the exact bytes that the
    // runtime receives, regardless of a stale/malformed client correlation.
    payload.source_fingerprint = payload.source.as_deref().map(crate::source_fingerprint);
    if state.simulation.is_none() {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "simulation runtime is unavailable".to_owned(),
        );
    }
    simulate_payload(state, payload).await
}
