impl DiagnosticStore {
    /// Returns the current wall-clock instant extrapolated from the latest
    /// paired host sample. External timestamps are epoch milliseconds, while
    /// execution ages and process-relative times remain monotonic.
    pub fn wall_clock_ms(&self) -> Option<i64> {
        let now = self.now().0;
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match inner.last_clock_sample {
            Some(sample) => {
                let utc = sample.utc_unix_ms?;
                let elapsed = i64::try_from(now.saturating_sub(sample.monotonic_ms.0)).ok()?;
                utc.checked_add(elapsed)
            }
            None => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|duration| i64::try_from(duration.as_millis()).ok()),
        }
    }
}
