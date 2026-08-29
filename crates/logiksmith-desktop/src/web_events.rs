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
