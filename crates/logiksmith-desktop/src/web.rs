//! Internal HTTP/SSE dashboard API and static asset server.

use crate::{
    ActivationRequest, AutomationDocument, AutomationEnvelope, FieldError, WebConfig,
    build_automation,
    diagnostics::{DiagnosticStore, DiagnosticUpdate, Replay, Snapshot},
    load_automation,
};
use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Json, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
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
        let _ = self.task.await;
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
    start_web_server_with_assets_and_activation(store, config, root, None).await
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
    start_web_server_with_assets_and_activation(store, config, &root, Some(activation)).await
}

async fn start_web_server_with_assets_and_activation(
    store: DiagnosticStore,
    config: WebConfig,
    root: &Path,
    activation: Option<mpsc::Sender<ActivationRequest>>,
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
        .route("/api/snapshot", get(snapshot))
        .route("/api/automation", get(get_automation).put(put_automation))
        .route("/api/events", get(events))
        .fallback_service(ServeDir::new(&root).not_found_service(ServeFile::new(index)))
        .with_state(AppState {
            store,
            automation_lock: Arc::new(Mutex::new(())),
            activation,
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

async fn get_automation(State(state): State<AppState>) -> Response {
    match load_automation(&state.store.automation_path()) {
        Ok((document, revision)) => {
            let snapshot = state.store.snapshot();
            (
                StatusCode::OK,
                Json(AutomationEnvelope {
                    document,
                    revision,
                    active_structural_revision: snapshot.logic.active_structural_revision,
                    saved_structural_revision: snapshot.logic.saved_structural_revision,
                    active_logic_revision: snapshot.logic.active_logic_revision,
                    saved_logic_revision: snapshot.logic.saved_logic_revision,
                    restart_required: snapshot.logic.restart_required,
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
    revision: u64,
}

#[derive(Debug, Serialize)]
struct SaveAutomationResponse {
    revision: u64,
    logic_activated: bool,
    active_logic_revision: u64,
    restart_required: bool,
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
    let candidate_logic_revision = crate::logic_revision(&request.document.logic.source);
    enum SaveOutcome {
        Conflict(AutomationDocument, u64),
        Invalid(Vec<FieldError>),
        Saved(Result<u64, String>),
    }
    // Keep the stale check and rename under one lock. The await below happens
    // only after this guard is dropped, so the axum handler remains Send.
    let save = {
        let _guard = state
            .automation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (current, current_revision) = match load_automation(&path) {
            Ok(value) => value,
            Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        };
        if request.revision != current_revision {
            SaveOutcome::Conflict(current, current_revision)
        } else if let Err(errors) = build_automation(request.document.clone()) {
            SaveOutcome::Invalid(errors)
        } else {
            SaveOutcome::Saved(atomic_save(&path, &request.document))
        }
    };
    match save {
        SaveOutcome::Conflict(current, current_revision) => {
            let snapshot = state.store.snapshot();
            (
                StatusCode::CONFLICT,
                Json(AutomationEnvelope {
                    document: current,
                    revision: current_revision,
                    active_structural_revision: snapshot.logic.active_structural_revision,
                    saved_structural_revision: snapshot.logic.saved_structural_revision,
                    active_logic_revision: snapshot.logic.active_logic_revision,
                    saved_logic_revision: snapshot.logic.saved_logic_revision,
                    restart_required: snapshot.logic.restart_required,
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
            Ok(revision) => {
                let active_structural_revision =
                    state.store.snapshot().logic.active_structural_revision;
                let mut logic_activated = false;
                let mut restart_required =
                    candidate_structural_revision != active_structural_revision;
                if !restart_required {
                    if let Some(activation) = &state.activation {
                        let (reply, result) = oneshot::channel();
                        let request = ActivationRequest {
                            source: request.document.logic.source.clone(),
                            revision: candidate_logic_revision,
                            reply,
                        };
                        if activation.send(request).await.is_ok() {
                            logic_activated = tokio::time::timeout(Duration::from_secs(2), result)
                                .await
                                .ok()
                                .and_then(Result::ok)
                                .is_some();
                        }
                    }
                    restart_required = !logic_activated;
                }
                state.store.set_saved_logic_state(
                    revision,
                    candidate_logic_revision,
                    candidate_structural_revision,
                    restart_required,
                );
                if logic_activated {
                    state.store.set_active_logic(
                        candidate_logic_revision,
                        request.document.logic.source.clone(),
                    );
                }
                let active_logic_revision = state.store.snapshot().logic.active_logic_revision;
                (
                    StatusCode::OK,
                    Json(SaveAutomationResponse {
                        revision,
                        logic_activated,
                        active_logic_revision,
                        restart_required,
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

fn atomic_save(path: &Path, document: &AutomationDocument) -> Result<u64, String> {
    let bytes = toml::to_string_pretty(document)
        .map_err(|error| error.to_string())?
        .into_bytes();
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
    result.map(|_| crate::automation_revision(&bytes))
}

#[derive(Debug, Deserialize)]
struct SinceQuery {
    since: Option<u64>,
}

#[derive(Debug, Serialize)]
struct UpdateData {
    revision: u64,
    snapshot: Snapshot,
}

#[derive(Debug, Serialize)]
struct ResyncData {
    revision: u64,
}

struct EventStreamState {
    initial: VecDeque<Event>,
    receiver: tokio::sync::broadcast::Receiver<DiagnosticUpdate>,
    store: DiagnosticStore,
}

async fn events(
    State(state): State<AppState>,
    Query(query): Query<SinceQuery>,
) -> impl IntoResponse {
    let subscription = state.store.subscribe(query.since);
    let mut initial = VecDeque::new();
    match subscription.replay {
        Replay::Updates(updates) => {
            initial.extend(updates.into_iter().map(update_event));
        }
        Replay::Resync { revision } => initial.push_back(resync_event(revision)),
    }
    let stream = event_stream(EventStreamState {
        initial,
        receiver: subscription.receiver,
        store: state.store,
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

fn event_stream(state: EventStreamState) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::unfold(state, |mut state| async move {
        if let Some(event) = state.initial.pop_front() {
            return Some((Ok(event), state));
        }
        match state.receiver.recv().await {
            Ok(update) => Some((Ok(update_event(update)), state)),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                Some((Ok(resync_event(state.store.latest_revision())), state))
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    })
}

fn update_event(update: DiagnosticUpdate) -> Event {
    let data = UpdateData {
        revision: update.revision,
        snapshot: update.snapshot,
    };
    Event::default()
        .event("update")
        .id(data.revision.to_string())
        .json_data(data)
        .unwrap_or_else(|_| Event::default().event("resync").data("{}"))
}

fn resync_event(revision: u64) -> Event {
    Event::default()
        .event("resync")
        .data(serde_json::to_string(&ResyncData { revision }).unwrap_or_else(|_| "{}".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::JOURNAL_CAPACITY;
    use std::{fs, net::IpAddr};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn store() -> DiagnosticStore {
        let runtime = crate::build_automation(crate::AutomationDocument {
            inputs: vec![
                crate::AutomationEndpoint {
                    name: "wall_switch".to_owned(),
                    dpt: "1.001".to_owned(),
                },
                crate::AutomationEndpoint {
                    name: "dimmer_level".to_owned(),
                    dpt: "5.001".to_owned(),
                },
            ],
            outputs: vec![
                crate::AutomationEndpoint {
                    name: "test_light".to_owned(),
                    dpt: "1.001".to_owned(),
                },
                crate::AutomationEndpoint {
                    name: "dimmer_output".to_owned(),
                    dpt: "5.001".to_owned(),
                },
            ],
            knx_bindings: vec![
                crate::KnxBinding {
                    endpoint: "wall_switch".to_owned(),
                    group_address: "2/2/52".to_owned(),
                },
                crate::KnxBinding {
                    endpoint: "dimmer_level".to_owned(),
                    group_address: "2/2/53".to_owned(),
                },
                crate::KnxBinding {
                    endpoint: "test_light".to_owned(),
                    group_address: "2/3/52".to_owned(),
                },
                crate::KnxBinding {
                    endpoint: "dimmer_output".to_owned(),
                    group_address: "2/3/53".to_owned(),
                },
            ],
            logic: crate::LogicDocument {
                source: "function handle(event, input) return nil end".to_owned(),
            },
        })
        .unwrap();
        DiagnosticStore::new(
            &runtime,
            std::env::temp_dir().join("logiksmith-web-test-automation.toml"),
            1,
        )
    }

    #[tokio::test]
    async fn missing_assets_are_a_startup_error() {
        let root =
            std::env::temp_dir().join(format!("logiksmith-no-assets-{}", std::process::id()));
        let error = start_web_server_with_assets(
            store(),
            WebConfig::new("127.0.0.1".parse::<IpAddr>().unwrap(), 1).unwrap(),
            &root,
        )
        .await;
        assert!(error.is_err());
    }

    #[tokio::test]
    async fn static_assets_and_snapshot_are_served() {
        let root = std::env::temp_dir().join(format!("logiksmith-assets-{}", std::process::id()));
        let _ = fs::create_dir_all(&root);
        fs::write(root.join("index.html"), "dashboard").unwrap();
        // Port zero is useful for an isolated test listener; file-based
        // configuration rejects it through `WebConfig::new`.
        let config = WebConfig {
            listen_ip: "127.0.0.1".parse().unwrap(),
            listen_port: 0,
        };
        let server = start_web_server_with_assets(store(), config, &root)
            .await
            .unwrap();
        let mut stream = tokio::net::TcpStream::connect(server.address)
            .await
            .unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        assert!(String::from_utf8_lossy(&response).contains("dashboard"));
        server.shutdown().await;
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn snapshot_endpoint_returns_the_complete_projection() {
        let root =
            std::env::temp_dir().join(format!("logiksmith-api-assets-{}", std::process::id()));
        let _ = fs::create_dir_all(&root);
        fs::write(root.join("index.html"), "dashboard").unwrap();
        let store = store();
        store.set_connection(crate::diagnostics::ConnectionState::Connected);
        let server = start_web_server_with_assets(
            store,
            WebConfig {
                listen_ip: "127.0.0.1".parse().unwrap(),
                listen_port: 0,
            },
            &root,
        )
        .await
        .unwrap();
        let response = raw_get(server.address, "/api/snapshot").await;
        assert!(response.contains("\"revision\":1"));
        assert!(response.contains("\"connection\":{"));
        assert!(response.contains("\"write\":{"));
        assert!(response.contains("\"status\":\"idle\""));
        assert!(response.contains("\"telegrams\":[]"));
        assert!(response.contains("\"logs\":[]"));
        server.shutdown().await;
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn events_replay_and_resync_when_journal_is_too_old() {
        let root =
            std::env::temp_dir().join(format!("logiksmith-sse-assets-{}", std::process::id()));
        let _ = fs::create_dir_all(&root);
        fs::write(root.join("index.html"), "dashboard").unwrap();
        let store = store();
        store.set_connection(crate::diagnostics::ConnectionState::Connected);
        let server = start_web_server_with_assets(
            store.clone(),
            WebConfig {
                listen_ip: "127.0.0.1".parse().unwrap(),
                listen_port: 0,
            },
            &root,
        )
        .await
        .unwrap();
        let replay = raw_get_prefix(server.address, "/api/events?since=0").await;
        assert!(replay.contains("event: update"));
        assert!(replay.contains("\"revision\":1"));
        for _ in 0..JOURNAL_CAPACITY + 1 {
            store.record_log("info", "test", "event", std::collections::BTreeMap::new());
        }
        let resync = raw_get_prefix(server.address, "/api/events?since=0").await;
        assert!(resync.contains("event: resync"));
        server.shutdown().await;
        let _ = fs::remove_dir_all(root);
    }

    async fn raw_get(address: std::net::SocketAddr, path: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8(response).unwrap()
    }

    async fn raw_get_prefix(address: std::net::SocketAddr, path: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = vec![0; 4096];
        let bytes = tokio::time::timeout(Duration::from_secs(1), stream.read(&mut response))
            .await
            .unwrap()
            .unwrap();
        String::from_utf8_lossy(&response[..bytes]).into_owned()
    }
}
