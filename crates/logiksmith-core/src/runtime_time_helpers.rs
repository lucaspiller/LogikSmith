impl Runtime {
    fn ensure_time(
    &self,
    block_id: Option<&BlockId>,
    now: MonotonicMs,
) -> Result<(), RuntimeEventError> {
    if let Some(previous) = self.last_accepted_at
        && now < previous
    {
        return Err(RuntimeEventError::TimeWentBackwards {
            block_id: block_id.cloned(),
            previous,
            current: now,
        });
    }
    Ok(())
}

fn ensure_schedule_time(&self, now: MonotonicMs) -> Result<(), TimeError> {
    if let Some(previous) = self.last_accepted_at
        && now < previous
    {
        return Err(TimeError::MonotonicWentBackwards {
            previous,
            current: now,
        });
    }
    Ok(())
}

/// Recomputes every schedule cursor strictly after `now_utc`, retaining a
/// previously delivered occurrence as a lower bound.
fn recompute_schedule_cursors(&mut self, now_utc: i64) {
    let structural_revision = self.schedule_structural_revision.unwrap_or(0);
    let schedules: Vec<(BlockId, ScheduleName, ScheduleRule)> = self
        .blocks
        .iter()
        .flat_map(|block| {
            block.config.schedules.iter().map(|block_schedule| {
                (
                    block.id().clone(),
                    block_schedule.name.clone(),
                    block_schedule.rule.clone(),
                )
            })
        })
        .collect();
    for (block_id, name, rule) in schedules {
        let key = (block_id, name);
        let cursor = self
            .schedule_cursors
            .entry(key)
            .or_insert_with(|| schedule::ScheduleCursor {
                last_delivered_utc_ms: None,
                next_occurrence_utc_ms: None,
                structural_revision,
                needs_rebaseline: true,
            });
        cursor.structural_revision = structural_revision;
        cursor.next_occurrence_utc_ms = next_occurrence_after_not_before(
            &rule,
            &self.site,
            now_utc,
            cursor.last_delivered_utc_ms,
        );
        cursor.needs_rebaseline = false;
    }
    }
}
