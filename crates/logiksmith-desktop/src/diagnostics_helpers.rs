fn first_block_revision(document: &crate::AutomationDocument) -> u64 {
    document
        .blocks
        .first()
        .map(|block| block.revision.max(1))
        .unwrap_or(1)
}

fn signal_for_causal_execution(
    inner: &Inner,
    producer_execution_id: u64,
    consumer_block_id: &str,
    trigger: &Trigger,
) -> Option<String> {
    let endpoint = match trigger {
        Trigger::Input(trigger) => trigger.endpoint.as_str(),
        _ => return None,
    };
    inner
        .blocks
        .values()
        .flat_map(|block| block.executions.iter())
        .find(|record| record.execution_id == producer_execution_id)
        .and_then(|record| {
            record.signal_effects.iter().find_map(|effect| {
                effect
                    .consumers
                    .iter()
                    .any(|consumer| {
                        consumer.block_id == consumer_block_id && consumer.endpoint == endpoint
                    })
                    .then_some(effect.signal.clone())
            })
        })
}
