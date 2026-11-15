fn normalized_persistent_snapshot(
    states: BTreeMap<logiksmith_core::BlockId, logiksmith_core::PersistentState>,
) -> BTreeMap<logiksmith_core::BlockId, logiksmith_core::PersistentState> {
    states
        .into_iter()
        .filter(|(_, state)| !state.is_empty())
        .collect()
}

impl DiagnosticStore {
    /// Loads the host-owned sidecar before the runtime accepts events.
    pub fn load_persistent_state(&self, runtime: &mut Runtime) -> Result<(), String> {
        if !runtime.persistent_state_enabled() {
            *self
                .persisted_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(BTreeMap::new());
            self.set_persistent_store_status("disabled", false, None);
            return Ok(());
        }

        let loaded = match self.state_store.load(&runtime.block_ids()) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.set_persistent_store_status("degraded", false, Some(error.clone()));
                return Err(error);
            }
        };
        runtime
            .load_persistent_state_snapshot(loaded.states.clone())
            .map_err(|error| {
                format!(
                    "persistent state sidecar {}: {error}",
                    self.state_store.path().display()
                )
            })?;

        let state = normalized_persistent_snapshot(loaded.states);
        *self
            .persisted_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            (!loaded.pruned).then(|| state.clone());
        self.set_persistent_store_status("healthy", false, None);
        if loaded.pruned && let Err(error) = self.persist_runtime_state(runtime) {
            tracing::error!(target: "logiksmith", error = %error, "pruned persistent state could not be saved");
        }
        Ok(())
    }

    /// Persists the complete live state after an accepted persistence effect.
    /// The cache avoids writes when a patch leaves the stored snapshot unchanged.
    pub fn persist_runtime_state(&self, runtime: &Runtime) -> Result<bool, String> {
        if !runtime.persistent_state_enabled() {
            self.set_persistent_store_status("disabled", false, None);
            return Ok(false);
        }
        let current = normalized_persistent_snapshot(runtime.persistent_state_snapshot());
        let previous = self
            .persisted_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if previous.as_ref() == Some(&current) {
            self.set_persistent_store_status("healthy", false, None);
            return Ok(false);
        }
        if let Err(error) = self.state_store.save(&current) {
            self.set_persistent_store_status("degraded", true, Some(error.clone()));
            return Err(error);
        }
        *self
            .persisted_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(current);
        self.set_persistent_store_status("healthy", false, None);
        Ok(true)
    }

    pub fn set_persistent_store_status(
        &self,
        status: &str,
        unsaved: bool,
        reason: Option<String>,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let next = PersistentStoreSnapshot {
            status: status.to_owned(),
            unsaved,
            reason,
        };
        if inner.persistent_store != next {
            inner.persistent_store = next;
            self.publish_locked(&mut inner);
        }
    }
}
