// Internal HTTP/SSE dashboard API and static asset server.

use crate::{
    ActivationRequest, AutomationBlock, AutomationBlockStatus, AutomationDocument,
    AutomationEnvelope, AutomationRuntime, CompiledCapabilities, FieldError, HostHealth, HostLimits, SimulationOutcome,
    SimulationPayload, SimulationRequest, WebConfig, WebhookInputRuntime, build_automation,
    diagnostics::{BlockSnapshot, DiagnosticStore, DiagnosticUpdate, Replay, Snapshot},
    external::ExternalInputMessage,
    load_automation, serialize_automation, structural_revision,
};
use axum::{
    Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, Json as ExtractJson, Path as AxumPath, Query, State,
        rejection::JsonRejection,
    },
    http::StatusCode,
    response::{
        IntoResponse, Json, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post, put},
};
#[cfg(feature = "webhook-inputs")]
use crate::external;
#[cfg(feature = "webhook-inputs")]
use axum::http::HeaderMap;
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    convert::Infallible,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time,
};
use tower_http::services::{ServeDir, ServeFile};

pub const STATIC_ASSET_ROOT: &str = "logiksmith-web/dist";

#[derive(Debug, Error)]
pub enum WebError {
    #[error(
        "frontend assets are unavailable at {path}; run ./scripts/bootstrap.sh or ./scripts/run-dev.sh"
    )]
    MissingAssets { path: PathBuf },
    #[error("failed to bind dashboard at {address}: {source}")]
    Bind {
        address: std::net::SocketAddr,
        source: std::io::Error,
    },
}

#[derive(Clone)]
struct AppState {
    store: DiagnosticStore,
    automation_lock: Arc<Mutex<()>>,
    activation: Option<mpsc::Sender<ActivationRequest>>,
    simulation: Option<mpsc::Sender<SimulationRequest>>,
    external: Option<mpsc::Sender<ExternalInputMessage>>,
    webhooks: Arc<std::collections::HashMap<String, WebhookInputRuntime>>,
    host: HostRuntimeState,
}

#[derive(Clone)]
struct HostRuntimeState {
    limits: HostLimits,
    health: HostHealth,
}

fn block_statuses(snapshot: &Snapshot) -> Vec<AutomationBlockStatus> {
    snapshot
        .blocks
        .iter()
        .map(|block| AutomationBlockStatus {
            id: block.id.clone(),
            active_enabled: block.active_enabled,
            saved_enabled: block.saved_enabled,
            active_revision: block.active_logic_revision,
            saved_revision: block.saved_logic_revision,
            active_logic_revision: block.active_logic_revision,
            saved_logic_revision: block.saved_logic_revision,
        })
        .collect()
}

pub struct WebServer {
    pub address: std::net::SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl WebServer {
    pub async fn shutdown(mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if time::timeout(Duration::from_secs(2), &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
            let _ = self.task.await;
        }
    }
}

pub async fn start_web_server(
    store: DiagnosticStore,
    config: WebConfig,
) -> Result<WebServer, WebError> {
    let relative = Path::new(STATIC_ASSET_ROOT);
    let root = if relative.is_dir() {
        relative.to_path_buf()
    } else {
        // Cargo integration tests run with the package directory as cwd;
        // retain the repository-relative path while making that invocation
        // behave like the desktop binary launched from the repository root.
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(STATIC_ASSET_ROOT)
    };
    start_web_server_with_assets(store, config, &root).await
}

/// Starts the internal dashboard server against an explicit asset directory.
/// The override keeps HTTP tests independent from the development checkout;
/// production always uses [`STATIC_ASSET_ROOT`].
pub async fn start_web_server_with_assets(
    store: DiagnosticStore,
    config: WebConfig,
    root: &Path,
) -> Result<WebServer, WebError> {
    start_web_server_with_assets_and_activation_with_host(
        store,
        config,
        root,
        None,
        None,
        None,
        Arc::new(Default::default()),
        HostLimits::desktop(),
        HostHealth::new(HostLimits::desktop()),
    )
    .await
}

/// Starts the dashboard with the runtime activation channel used for
/// source-only saves. Tests and callers that do not run a session may omit it;
/// those saves remain restart-pending after persistence.
pub async fn start_web_server_with_activation(
    store: DiagnosticStore,
    config: WebConfig,
    activation: mpsc::Sender<ActivationRequest>,
) -> Result<WebServer, WebError> {
    let relative = Path::new(STATIC_ASSET_ROOT);
    let root = if relative.is_dir() {
        relative.to_path_buf()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(STATIC_ASSET_ROOT)
    };
    start_web_server_with_assets_and_activation_with_host(
        store,
        config,
        &root,
        Some(activation),
        None,
        None,
        Arc::new(Default::default()),
        HostLimits::desktop(),
        HostHealth::new(HostLimits::desktop()),
    )
    .await
}

/// Starts the dashboard with the runtime-owned source activation and
/// immutable simulation request channels.
pub async fn start_web_server_with_runtime(
    store: DiagnosticStore,
    config: WebConfig,
    activation: mpsc::Sender<ActivationRequest>,
    simulation: mpsc::Sender<SimulationRequest>,
) -> Result<WebServer, WebError> {
    start_web_server_with_assets_and_activation_with_host(
        store,
        config,
        &root_for_assets(),
        Some(activation),
        Some(simulation),
        None,
        Arc::new(Default::default()),
        HostLimits::desktop(),
        HostHealth::new(HostLimits::desktop()),
    )
    .await
}

/// Starts the dashboard with runtime channels and configured webhook inputs.
/// The webhook route only submits typed values to the serial runtime owner.
pub async fn start_web_server_with_runtime_and_sources(
    store: DiagnosticStore,
    config: WebConfig,
    activation: mpsc::Sender<ActivationRequest>,
    simulation: mpsc::Sender<SimulationRequest>,
    automation: &AutomationRuntime,
    external: mpsc::Sender<ExternalInputMessage>,
) -> Result<WebServer, WebError> {
    start_web_server_with_runtime_and_sources_with_host(
        store,
        config,
        activation,
        simulation,
        automation,
        external,
        HostLimits::desktop(),
        HostHealth::new(HostLimits::desktop()),
    )
    .await
}

/// Starts the dashboard with an explicitly selected host profile and health
/// state.  Runtime channels are still owned and drained by the host session;
/// this function only admits bounded requests and exposes status endpoints.
pub async fn start_web_server_with_runtime_and_sources_with_host(
    store: DiagnosticStore,
    config: WebConfig,
    activation: mpsc::Sender<ActivationRequest>,
    simulation: mpsc::Sender<SimulationRequest>,
    automation: &AutomationRuntime,
    external: mpsc::Sender<ExternalInputMessage>,
    limits: HostLimits,
    health: HostHealth,
) -> Result<WebServer, WebError> {
    let relative = Path::new(STATIC_ASSET_ROOT);
    let root = if relative.is_dir() {
        relative.to_path_buf()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(STATIC_ASSET_ROOT)
    };
    start_web_server_with_assets_and_activation_with_host(
        store,
        config,
        &root,
        Some(activation),
        Some(simulation),
        Some(external),
        Arc::new(
            automation
                .webhook_inputs
                .iter()
                .cloned()
                .map(|source| (source.name.clone(), source))
                .collect(),
        ),
        limits,
        health,
    )
    .await
}

fn root_for_assets() -> PathBuf {
    let relative = Path::new(STATIC_ASSET_ROOT);
    if relative.is_dir() {
        relative.to_path_buf()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(STATIC_ASSET_ROOT)
    }
}

/// Compatibility helper retained for the in-crate HTTP tests and callers
/// which do not need to share a runtime health object.
#[allow(dead_code)]
async fn start_web_server_with_assets_and_activation(
    store: DiagnosticStore,
    config: WebConfig,
    root: &Path,
    activation: Option<mpsc::Sender<ActivationRequest>>,
    simulation: Option<mpsc::Sender<SimulationRequest>>,
    external: Option<mpsc::Sender<ExternalInputMessage>>,
    webhooks: Arc<std::collections::HashMap<String, WebhookInputRuntime>>,
) -> Result<WebServer, WebError> {
    start_web_server_with_assets_and_activation_with_host(
        store,
        config,
        root,
        activation,
        simulation,
        external,
        webhooks,
        HostLimits::desktop(),
        HostHealth::new(HostLimits::desktop()),
    )
    .await
}

async fn start_web_server_with_assets_and_activation_with_host(
    store: DiagnosticStore,
    config: WebConfig,
    root: &Path,
    activation: Option<mpsc::Sender<ActivationRequest>>,
    simulation: Option<mpsc::Sender<SimulationRequest>>,
    external: Option<mpsc::Sender<ExternalInputMessage>>,
    webhooks: Arc<std::collections::HashMap<String, WebhookInputRuntime>>,
    limits: HostLimits,
    health: HostHealth,
) -> Result<WebServer, WebError> {
    let root = root.to_path_buf();
    let index = root.join("index.html");
    if !root.is_dir() || !index.is_file() {
        return Err(WebError::MissingAssets { path: root });
    }
    let address = config.socket_addr();
    if !config.listen_ip.is_loopback() {
        tracing::warn!(
            target: "logiksmith.web",
            address = %address,
            "WARNING: dashboard is listening on a non-loopback address; it has no authentication or TLS"
        );
    }
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|source| WebError::Bind { address, source })?;
    let address = listener
        .local_addr()
        .map_err(|source| WebError::Bind { address, source })?;
    let router = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api/snapshot", get(snapshot))
        .route("/api/automation", get(get_automation).put(put_automation))
        .route("/api/simulate", post(simulate))
        .route(
            "/api/blocks/{block_id}/validate",
            post(validate_block).layer(DefaultBodyLimit::max(crate::MAX_HTTP_BODY_BYTES)),
        )
        .route(
            "/api/blocks/{block_id}/simulate",
            post(simulate_block).layer(DefaultBodyLimit::max(crate::MAX_HTTP_BODY_BYTES)),
        )
        .route(
            "/api/blocks/{block_id}/source",
            put(activate_block_source).layer(DefaultBodyLimit::max(crate::MAX_HTTP_BODY_BYTES)),
        )
        .route(
            "/api/blocks/{block_id}/enabled",
            put(set_block_enabled).layer(DefaultBodyLimit::max(crate::MAX_HTTP_BODY_BYTES)),
        )
        .route(
            "/api/blocks/{block_id}/resume",
            post(resume_block).layer(DefaultBodyLimit::max(crate::MAX_HTTP_BODY_BYTES)),
        )
        .route("/api/schedules/preview", post(preview_schedule))
        .route("/api/schedules/simulate", post(simulate_schedule));
    #[cfg(feature = "webhook-inputs")]
    let router = router.route(
        "/api/webhooks/{source}",
        post(webhook).layer(DefaultBodyLimit::max(crate::MAX_HTTP_BODY_BYTES)),
    );
    #[cfg(not(feature = "webhook-inputs"))]
    let router = router.route(
        "/api/webhooks/{source}",
        post(webhook_disabled).layer(DefaultBodyLimit::max(crate::MAX_HTTP_BODY_BYTES)),
    );
    let router = router
        .route("/api/events", get(events))
        .fallback_service(ServeDir::new(&root).not_found_service(ServeFile::new(index)))
        .with_state(AppState {
            store,
            automation_lock: Arc::new(Mutex::new(())),
            activation,
            simulation,
            external,
            webhooks,
            host: HostRuntimeState { limits, health },
        });
    let (sender, receiver) = oneshot::channel();
    let task = tokio::spawn(async move {
        let result = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = receiver.await;
            })
            .await;
        if let Err(error) = result {
            tracing::error!(target: "logiksmith.web", error = %error, "dashboard server stopped with an error");
        }
    });
    Ok(WebServer {
        address,
        shutdown: Some(sender),
        task,
    })
}

async fn snapshot(State(state): State<AppState>) -> Json<Snapshot> {
    Json(state.store.snapshot())
}

#[cfg(not(feature = "webhook-inputs"))]
async fn webhook_disabled() -> StatusCode {
    // Keep a stable 404 for callers which retained an old route, while
    // compiling out webhook parsing and delivery logic.
    StatusCode::NOT_FOUND
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    ready: bool,
    profile: &'static str,
    fatal: Option<String>,
    capabilities: CompiledCapabilities,
}

async fn healthz(State(state): State<AppState>) -> Response {
    let health = state.host.health.snapshot();
    let status_code = if health.fatal.is_some() {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    (
        status_code,
        Json(HealthResponse {
            status: if health.fatal.is_some() {
                "failed"
            } else {
                "ok"
            },
            ready: health.ready,
            profile: health.profile.as_str(),
            fatal: health.fatal,
            capabilities: crate::capabilities::compiled_capabilities(),
        }),
    )
        .into_response()
}

async fn readyz(State(state): State<AppState>) -> Response {
    let health = state.host.health.snapshot();
    let ready = health.ready && health.fatal.is_none();
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(HealthResponse {
            status: if ready {
                "ready"
            } else if health.fatal.is_some() {
                "failed"
            } else {
                "starting"
            },
            ready,
            profile: health.profile.as_str(),
            fatal: health.fatal,
            capabilities: crate::capabilities::compiled_capabilities(),
        }),
    )
        .into_response()
}

#[cfg(feature = "webhook-inputs")]
async fn webhook(
    AxumPath(source_name): AxumPath<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(source) = state.webhooks.get(&source_name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    {
        state.store.record_webhook_rejected(&source_name);
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    if body.len() > crate::MAX_HTTP_BODY_BYTES {
        state.store.record_webhook_rejected(&source_name);
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    if !external::webhook_authorized(
        source,
        headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
    ) {
        state.store.record_webhook_rejected(&source_name);
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let value = match external::parse_webhook_value(source, &body) {
        Ok(value) => value,
        Err(external::ExternalError::BodyTooLarge) => {
            state.store.record_webhook_rejected(&source_name);
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
        Err(external::ExternalError::MissingPointer(_))
        | Err(external::ExternalError::InvalidValue(_)) => {
            state.store.record_webhook_rejected(&source_name);
            return StatusCode::UNPROCESSABLE_ENTITY.into_response();
        }
        Err(external::ExternalError::InvalidJson(_)) => {
            state.store.record_webhook_rejected(&source_name);
            return StatusCode::BAD_REQUEST.into_response();
        }
        Err(_) => {
            state.store.record_webhook_rejected(&source_name);
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    let Some(sender) = state.external.clone() else {
        state.store.record_webhook_rejected(&source_name);
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let (reply, response) = tokio::sync::oneshot::channel();
    let message = ExternalInputMessage {
        source: source_name.clone(),
        update: external::ExternalInputUpdate::Trigger(value),
        kind: external::ExternalInputKind::Webhook {
            source: source_name.clone(),
        },
        reply: Some(reply),
    };
    match sender.try_send(message) {
        Ok(()) => {
            if state.host.health.snapshot().ready {
                let depth = state
                    .host
                    .limits
                    .external_input_queue
                    .saturating_sub(sender.capacity());
                state.store.record_queue_admitted("external_input", depth);
            }
        }
        Err(tokio::sync::mpsc::error::TrySendError::Full(_message)) => {
            state.store.record_webhook_rejected(&source_name);
            state.store.record_queue_rejected(
                "external_input",
                state.host.limits.external_input_queue,
                true,
            );
            state.store.record_runtime_fatal(
                format!(
                    "runtime overload in external input queue (capacity={}, depth={})",
                    state.host.limits.external_input_queue,
                    state.host.limits.external_input_queue,
                ),
                true,
            );
            state.host.health.fail(format!(
                "runtime overload in external input queue (capacity={}, depth={})",
                state.host.limits.external_input_queue, state.host.limits.external_input_queue,
            ));
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_message)) => {
            state.store.record_webhook_rejected(&source_name);
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    match tokio::time::timeout(Duration::from_secs(10), response).await {
        Ok(Ok(Ok(()))) => {
            state.store.record_webhook_accepted(&source_name, value);
            StatusCode::ACCEPTED.into_response()
        }
        _ => {
            state.store.record_webhook_rejected(&source_name);
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}
