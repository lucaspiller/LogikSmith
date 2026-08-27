//! Internal HTTP/SSE dashboard API and static asset server.

use crate::{
    WebConfig,
    diagnostics::{DiagnosticStore, DiagnosticUpdate, Replay, Snapshot},
};
use axum::{
    Router,
    extract::{Query, State},
    response::{
        IntoResponse, Json,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    convert::Infallible,
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;
use tokio::{sync::oneshot, task::JoinHandle};
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
        .route("/api/events", get(events))
        .fallback_service(ServeDir::new(&root).not_found_service(ServeFile::new(index)))
        .with_state(AppState { store });
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
    use logiksmith_core::{Dpt, EngineConfig};
    use std::{fs, net::IpAddr};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn store() -> DiagnosticStore {
        DiagnosticStore::new(EngineConfig {
            input_group_address: "2/2/52".parse().unwrap(),
            input_dpt: Dpt::BOOL,
            output_group_address: "2/3/52".parse().unwrap(),
            output_dpt: Dpt::BOOL,
            off_delay_ms: 5_000,
        })
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
