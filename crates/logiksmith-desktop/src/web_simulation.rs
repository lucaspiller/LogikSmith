#[derive(Debug, Serialize)]
struct SimulationConflictResponse {
    error: String,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    current_logic_revision: u64,
}

#[derive(Debug, Serialize)]
struct ScheduleSimulationConflictResponse {
    error: String,
    /// The active persisted revision of the requested block.
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    current_revision: u64,
    /// The active structural revision shared by the automation document.
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    current_structural_revision: u64,
}

#[derive(Debug, Serialize)]
struct BlockSimulationConflictResponse {
    error: String,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    current_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    current_structural_revision: u64,
}

/// Schedule previews have a dedicated request so the browser does not need
/// to fabricate input values, pending timers, or an internal Lua source hash.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchedulePreviewRequest {
    block_id: String,
    schedule: String,
    #[serde(default, alias = "afterUtcMs")]
    after_utc_ms: Option<i64>,
    #[serde(default)]
    count: Option<usize>,
}

/// A selected schedule occurrence is simulated with the public persisted
/// block revision. The desktop maps it to the core's private source hash.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleSimulationRequest {
    block_id: String,
    schedule: String,
    #[serde(alias = "occurrenceAtUtcMs")]
    occurrence_at_utc_ms: i64,
    #[serde(alias = "expectedRevision")]
    #[serde(deserialize_with = "crate::wire_revision::deserialize")]
    expected_revision: u64,
    #[serde(alias = "expectedStructuralRevision")]
    #[serde(deserialize_with = "crate::wire_revision::deserialize")]
    expected_structural_revision: u64,
}

fn schedule_payload(
    block_id: String,
    schedule: String,
    expected_revision: u64,
    expected_structural_revision: Option<u64>,
    occurrence_at_utc_ms: Option<i64>,
    preview_after_utc_ms: Option<i64>,
    preview_count: Option<usize>,
) -> SimulationPayload {
    SimulationPayload {
        block_id,
        source: None,
        source_fingerprint: None,
        expected_logic_revision: expected_revision,
        expected_structural_revision,
        preview_after_utc_ms,
        preview_count,
        trigger: crate::SimulationTriggerPayload {
            trigger_type: Some("schedule".to_owned()),
            endpoint: None,
            value: None,
            previous: None,
            name: None,
            fired_at_ms: None,
            schedule: Some(schedule),
            occurrence_at_ms: occurrence_at_utc_ms,
        },
        inputs: Vec::new(),
        state: None,
        pending_timers: None,
    }
}

async fn simulate(
    State(state): State<AppState>,
    payload: Result<Json<SimulationPayload>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(error) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(FieldErrorsResponse {
                    errors: vec![FieldError {
                        path: "request".to_owned(),
                        message: error.to_string(),
                    }],
                }),
            )
                .into_response();
        }
    };
    simulate_payload(state, payload).await
}

async fn preview_schedule(
    State(state): State<AppState>,
    payload: Result<ExtractJson<SchedulePreviewRequest>, JsonRejection>,
) -> Response {
    let ExtractJson(request) = match payload {
        Ok(payload) => payload,
        Err(error) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(FieldErrorsResponse {
                    errors: vec![FieldError {
                        path: "request".to_owned(),
                        message: error.to_string(),
                    }],
                }),
            )
                .into_response();
        }
    };
    if request
        .count
        .is_some_and(|count| !(1..=10).contains(&count))
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(FieldErrorsResponse {
                errors: vec![FieldError {
                    path: "count".to_owned(),
                    message: "must be between 1 and 10".to_owned(),
                }],
            }),
        )
            .into_response();
    }
    let expected_revision = state
        .store
        .active_block_revision(&request.block_id)
        .unwrap_or(1);
    simulate_payload(
        state,
        schedule_payload(
            request.block_id,
            request.schedule,
            expected_revision,
            None,
            None,
            request.after_utc_ms,
            request.count,
        ),
    )
    .await
}

async fn simulate_schedule(
    State(state): State<AppState>,
    payload: Result<ExtractJson<ScheduleSimulationRequest>, JsonRejection>,
) -> Response {
    let ExtractJson(request) = match payload {
        Ok(payload) => payload,
        Err(error) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(FieldErrorsResponse {
                    errors: vec![FieldError {
                        path: "request".to_owned(),
                        message: error.to_string(),
                    }],
                }),
            )
                .into_response();
        }
    };
    simulate_payload(
        state,
        schedule_payload(
            request.block_id,
            request.schedule,
            request.expected_revision,
            Some(request.expected_structural_revision),
            Some(request.occurrence_at_utc_ms),
            None,
            None,
        ),
    )
    .await
}

async fn simulate_payload(state: AppState, payload: SimulationPayload) -> Response {
    let Some(simulation) = state.simulation else {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "simulation runtime is unavailable".to_owned(),
        );
    };
    let (reply, result) = oneshot::channel();
    match simulation.try_send(SimulationRequest { payload, reply }) {
        Ok(()) => {
            if state.host.health.snapshot().ready {
                let depth = state
                    .host
                    .limits
                    .simulation_queue
                    .saturating_sub(simulation.capacity());
                state.store.record_queue_admitted("simulation", depth);
            }
        }
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            state.store.record_queue_rejected(
                "simulation",
                state.host.limits.simulation_queue,
                true,
            );
            state.store.record_runtime_fatal(
                format!(
                    "runtime overload in simulation queue (capacity={}, depth={})",
                    state.host.limits.simulation_queue,
                    state.host.limits.simulation_queue,
                ),
                true,
            );
            state.host.health.fail(format!(
                "runtime overload in simulation queue (capacity={}, depth={})",
                state.host.limits.simulation_queue,
                state.host.limits.simulation_queue,
            ));
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "runtime overload; container will restart".to_owned(),
            );
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "simulation runtime is unavailable".to_owned(),
            );
        }
    }
    let result = match time::timeout(Duration::from_secs(2), result).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) | Err(_) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "simulation runtime did not respond".to_owned(),
            );
        }
    };
    match result {
        SimulationOutcome::Complete(result) => (StatusCode::OK, Json(result)).into_response(),
        SimulationOutcome::Previews(preview) => (StatusCode::OK, Json(preview)).into_response(),
        SimulationOutcome::NotFound => {
            json_error(StatusCode::NOT_FOUND, "unknown logic block".to_owned())
        }
        SimulationOutcome::ScheduleNotFound => {
            json_error(StatusCode::NOT_FOUND, "unknown schedule".to_owned())
        }
        SimulationOutcome::Invalid(errors) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(FieldErrorsResponse { errors }),
        )
            .into_response(),
        SimulationOutcome::Conflict { current_revision } => (
            StatusCode::CONFLICT,
            Json(SimulationConflictResponse {
                error: "active logic revision changed; refresh and run the simulation again"
                    .to_owned(),
                current_logic_revision: current_revision,
            }),
        )
            .into_response(),
        SimulationOutcome::ScheduleConflict {
            current_revision,
            current_structural_revision,
        } => (
            StatusCode::CONFLICT,
            Json(ScheduleSimulationConflictResponse {
                error: "active schedule or block revision changed; refresh and run the schedule simulation again"
                    .to_owned(),
                current_revision,
                current_structural_revision,
            }),
        )
            .into_response(),
        SimulationOutcome::BlockConflict {
            current_revision,
            current_structural_revision,
        } => (
            StatusCode::CONFLICT,
            Json(BlockSimulationConflictResponse {
                error: "active block or structural revision changed; refresh and run the draft simulation again"
                    .to_owned(),
                current_revision,
                current_structural_revision,
            }),
        )
            .into_response(),
    }
}
