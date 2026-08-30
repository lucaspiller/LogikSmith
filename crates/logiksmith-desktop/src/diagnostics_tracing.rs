static ACTIVE_STORE: OnceLock<Arc<Mutex<Option<DiagnosticStore>>>> = OnceLock::new();

pub fn activate_tracing_store(store: DiagnosticStore) {
    let slot = ACTIVE_STORE.get_or_init(|| Arc::new(Mutex::new(None)));
    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(store);
}

pub fn tracing_layer() -> DiagnosticLayer {
    DiagnosticLayer {
        slot: ACTIVE_STORE
            .get_or_init(|| Arc::new(Mutex::new(None)))
            .clone(),
    }
}
pub struct DiagnosticLayer {
    slot: Arc<Mutex<Option<DiagnosticStore>>>,
}
impl<S> Layer<S> for DiagnosticLayer
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let store = self
            .slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let Some(store) = store else { return };
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        store.record_log(
            event.metadata().level().to_string().to_lowercase(),
            event.metadata().target().to_owned(),
            visitor.message.unwrap_or_default(),
            visitor.fields,
        );
    }
}
#[derive(Default)]
struct FieldVisitor {
    message: Option<String>,
    fields: BTreeMap<String, String>,
}
impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let value = format!("{value:?}");
        if field.name() == "message" {
            self.message = Some(value.trim_matches('"').to_owned());
        } else {
            self.fields.insert(field.name().to_owned(), value);
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        } else {
            self.fields
                .insert(field.name().to_owned(), value.to_owned());
        }
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.record_str(field, &value.to_string());
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.record_str(field, &value.to_string());
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.record_str(field, &value.to_string());
    }
    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        self.record_str(field, &value.to_string());
    }
}
