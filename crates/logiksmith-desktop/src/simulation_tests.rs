    #[test]
    fn simulated_timer_document_revisions_are_mapped_to_the_core_revision() {
        let payload: SimulationPayload = serde_json::from_value(serde_json::json!({
            "block_id": "test",
            "expected_logic_revision": "6",
            "trigger": {},
            "inputs": [],
            "pending_timers": [{
                "name": "off",
                "scheduled_at_ms": 1000,
                "due_at_ms": 6000,
                "logic_revision": "6"
            }]
        }))
        .unwrap();

        let timers = simulation_pending_timers(&payload, None, 6, u64::MAX).unwrap();
        assert_eq!(timers[0].scheduled_logic_revision, u64::MAX);
    }
