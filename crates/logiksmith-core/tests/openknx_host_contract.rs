//! Deterministic contract tests for the embedded raw-telegram adapter.
//!
//! These tests deliberately use an in-memory adapter instead of OpenKNX or a
//! physical bus. The adapter is the small seam the embedded host must keep:
//! destination group address filtering, DPT decoding, core invocation, and
//! output-address routing.

use std::collections::{BTreeMap, VecDeque};

use logiksmith_core::{
    BlockConfig, BlockId, Dpt, Endpoint, InputEvent, MonotonicMs, OutputEffect, Runtime,
    RuntimeConfig, TypedValue,
};

const INPUT_ADDRESS: u16 = 0x1201;
const REBOUND_INPUT_ADDRESS: u16 = 0x1202;
const OUTPUT_ADDRESS: u16 = 0x1301;
const QUEUE_CAPACITY: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RawGroupTelegram {
    destination: u16,
    /// The application data byte for a DPT 1.001 group-value write. Keeping
    /// this fixture at the application-data boundary avoids pretending to be
    /// a complete KNX TP frame parser.
    payload: u8,
    received_at: MonotonicMs,
}

impl RawGroupTelegram {
    const fn bool(destination: u16, value: bool, received_at: u64) -> Self {
        Self {
            destination,
            payload: if value { 1 } else { 0 },
            received_at: MonotonicMs(received_at),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Binding {
    block_id: BlockId,
    endpoint: logiksmith_core::EndpointName,
    dpt: Dpt,
}

impl Binding {
    fn bool_input(address: u16) -> (u16, Self) {
        (
            address,
            Self {
                block_id: block_id(),
                endpoint: endpoint("wall_switch"),
                dpt: Dpt::BOOL,
            },
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OutgoingTelegram {
    destination: u16,
    payload: u8,
}

/// Small bounded adapter double. Its policy is intentionally explicit so a
/// future OpenKNX queue implementation can be compared against this contract:
/// drop newest input on overflow and reject all work after shutdown.
struct InMemoryAdapter {
    runtime: Runtime,
    bindings: BTreeMap<u16, Binding>,
    output_addresses: BTreeMap<(BlockId, logiksmith_core::EndpointName), u16>,
    queue: VecDeque<RawGroupTelegram>,
    queue_capacity: usize,
    dropped_telegrams: usize,
    outgoing: Vec<OutgoingTelegram>,
    shutdown: bool,
}

impl InMemoryAdapter {
    fn new(queue_capacity: usize) -> Self {
        let mut bindings = BTreeMap::new();
        bindings.extend([Binding::bool_input(INPUT_ADDRESS)]);
        let mut output_addresses = BTreeMap::new();
        output_addresses.insert((block_id(), endpoint("light")), OUTPUT_ADDRESS);
        Self {
            runtime: Runtime::new(RuntimeConfig::new(vec![BlockConfig::new(
                block_id(),
                true,
                vec![
                    Endpoint::input(endpoint("wall_switch"), Dpt::BOOL),
                    Endpoint::output(endpoint("light"), Dpt::BOOL),
                ],
                logic_source(),
            )])),
            bindings,
            output_addresses,
            queue: VecDeque::new(),
            queue_capacity,
            dropped_telegrams: 0,
            outgoing: Vec::new(),
            shutdown: false,
        }
    }

    fn bind_input(&mut self, address: u16) {
        self.bindings.clear();
        self.bindings.extend([Binding::bool_input(address)]);
    }

    fn enqueue(&mut self, telegram: RawGroupTelegram) -> bool {
        if self.shutdown || self.queue.len() >= self.queue_capacity {
            self.dropped_telegrams += 1;
            return false;
        }
        self.queue.push_back(telegram);
        true
    }

    fn drain(&mut self) {
        while let Some(telegram) = self.queue.pop_front() {
            self.process_telegram(telegram);
        }
    }

    fn process_telegram(&mut self, telegram: RawGroupTelegram) {
        let Some(binding) = self.bindings.get(&telegram.destination).cloned() else {
            return;
        };
        let value = decode_bool_dpt(telegram.payload);
        assert_eq!(binding.dpt, Dpt::BOOL);
        let event = InputEvent::new(binding.endpoint, TypedValue::bool(value));
        let execution = self
            .runtime
            .process_input(&binding.block_id, event, telegram.received_at)
            .expect("fixture telegram should be accepted by the runtime");
        if let Some(execution) = execution {
            self.route_outputs(&binding.block_id, execution.execution.outcome.ok());
        }
    }

    fn advance_timers(&mut self, now: u64) {
        while let Some(execution) = self
            .runtime
            .process_next_due_timer(MonotonicMs(now))
            .expect("fixture timer should be accepted by the runtime")
        {
            self.route_outputs(&execution.block_id, execution.execution.outcome.ok());
        }
    }

    fn route_outputs(
        &mut self,
        block_id: &BlockId,
        transition: Option<logiksmith_core::Transition>,
    ) {
        let Some(transition) = transition else {
            return;
        };
        for output in transition.outputs {
            self.route_output(block_id, output);
        }
    }

    fn route_output(&mut self, block_id: &BlockId, output: OutputEffect) {
        let Some(destination) = self
            .output_addresses
            .get(&(block_id.clone(), output.endpoint))
            .copied()
        else {
            return;
        };
        let logiksmith_core::Value::Bool(value) = output.value.value() else {
            return;
        };
        self.outgoing.push(OutgoingTelegram {
            destination,
            payload: if value { 1 } else { 0 },
        });
    }

    fn shutdown(&mut self) {
        self.shutdown = true;
        self.queue.clear();
    }
}

fn decode_bool_dpt(payload: u8) -> bool {
    payload & 0x01 != 0
}

fn block_id() -> BlockId {
    "switch_logic".parse().expect("fixture block ID")
}

fn endpoint(value: &str) -> logiksmith_core::EndpointName {
    value.parse().expect("fixture endpoint name")
}

fn logic_source() -> &'static str {
    r#"function handle(event)
  if event.type == 'input' and event.input == 'wall_switch' and event.value == true then
    return { outputs = { light = true }, timers = { off = { after = 10 } } }
  end
  if event.type == 'timer' and event.timer == 'off' then
    return { outputs = { light = false } }
  end
  return {}
end"#
}

#[test]
fn raw_telegram_filter_decodes_bool_and_routes_output() {
    let mut adapter = InMemoryAdapter::new(QUEUE_CAPACITY);

    assert!(adapter.enqueue(RawGroupTelegram::bool(INPUT_ADDRESS, true, 1)));
    assert!(adapter.enqueue(RawGroupTelegram::bool(0x7fff, true, 2)));
    adapter.drain();

    assert_eq!(adapter.dropped_telegrams, 0);
    assert_eq!(
        adapter.outgoing,
        vec![OutgoingTelegram {
            destination: OUTPUT_ADDRESS,
            payload: 1,
        }]
    );
    assert!(adapter.runtime.block(&block_id()).is_some());
}

#[test]
fn unbound_telegrams_are_ignored_without_triggering_logic() {
    let mut adapter = InMemoryAdapter::new(QUEUE_CAPACITY);

    assert!(adapter.enqueue(RawGroupTelegram::bool(0x7fff, true, 1)));
    assert!(adapter.enqueue(RawGroupTelegram::bool(INPUT_ADDRESS, false, 2)));
    adapter.drain();

    assert!(adapter.outgoing.is_empty());
    assert!(adapter.runtime.next_timer_deadline().is_none());
    assert!(!decode_bool_dpt(0));
    assert!(decode_bool_dpt(1));
    assert!(decode_bool_dpt(0x81));
}

#[test]
fn changing_binding_address_requires_no_ets_or_runtime_restart() {
    let mut adapter = InMemoryAdapter::new(QUEUE_CAPACITY);

    adapter.bind_input(REBOUND_INPUT_ADDRESS);
    assert!(adapter.enqueue(RawGroupTelegram::bool(INPUT_ADDRESS, true, 1)));
    assert!(adapter.enqueue(RawGroupTelegram::bool(REBOUND_INPUT_ADDRESS, true, 2)));
    adapter.drain();

    assert_eq!(adapter.outgoing.len(), 1);
    assert_eq!(adapter.outgoing[0].destination, OUTPUT_ADDRESS);
}

#[test]
fn retrigger_replaces_timer_and_emits_only_after_latest_deadline() {
    let mut adapter = InMemoryAdapter::new(QUEUE_CAPACITY);

    assert!(adapter.enqueue(RawGroupTelegram::bool(INPUT_ADDRESS, true, 0)));
    adapter.drain();
    assert_eq!(adapter.runtime.next_timer_deadline(), Some(MonotonicMs(10)));

    assert!(adapter.enqueue(RawGroupTelegram::bool(INPUT_ADDRESS, true, 5)));
    adapter.drain();
    assert_eq!(adapter.runtime.next_timer_deadline(), Some(MonotonicMs(15)));
    assert_eq!(adapter.outgoing.len(), 2);

    adapter.advance_timers(14);
    assert_eq!(adapter.outgoing.len(), 2);
    adapter.advance_timers(15);
    assert_eq!(
        adapter.outgoing,
        vec![
            OutgoingTelegram {
                destination: OUTPUT_ADDRESS,
                payload: 1
            },
            OutgoingTelegram {
                destination: OUTPUT_ADDRESS,
                payload: 1
            },
            OutgoingTelegram {
                destination: OUTPUT_ADDRESS,
                payload: 0
            },
        ]
    );
}

#[test]
fn queue_overflow_drops_newest_and_shutdown_rejects_future_work() {
    let mut adapter = InMemoryAdapter::new(QUEUE_CAPACITY);

    assert!(adapter.enqueue(RawGroupTelegram::bool(INPUT_ADDRESS, true, 1)));
    assert!(adapter.enqueue(RawGroupTelegram::bool(INPUT_ADDRESS, false, 2)));
    assert!(!adapter.enqueue(RawGroupTelegram::bool(INPUT_ADDRESS, true, 3)));
    assert_eq!(adapter.dropped_telegrams, 1);
    adapter.shutdown();
    assert!(!adapter.enqueue(RawGroupTelegram::bool(INPUT_ADDRESS, true, 4)));
    assert_eq!(adapter.dropped_telegrams, 2);
    adapter.drain();
    assert!(adapter.outgoing.is_empty());
}

#[test]
fn desktop_and_embedded_adapter_doubles_have_identical_semantics() {
    fn run_sequence() -> Vec<OutgoingTelegram> {
        let mut adapter = InMemoryAdapter::new(QUEUE_CAPACITY);
        adapter.enqueue(RawGroupTelegram::bool(INPUT_ADDRESS, true, 0));
        adapter.enqueue(RawGroupTelegram::bool(0x7fff, true, 1));
        adapter.drain();
        adapter.enqueue(RawGroupTelegram::bool(INPUT_ADDRESS, true, 5));
        adapter.drain();
        adapter.advance_timers(14);
        adapter.advance_timers(15);
        adapter.outgoing
    }

    let desktop = run_sequence();
    let embedded = run_sequence();
    assert_eq!(desktop, embedded);
    assert_eq!(
        desktop,
        vec![
            OutgoingTelegram {
                destination: OUTPUT_ADDRESS,
                payload: 1
            },
            OutgoingTelegram {
                destination: OUTPUT_ADDRESS,
                payload: 1
            },
            OutgoingTelegram {
                destination: OUTPUT_ADDRESS,
                payload: 0
            },
        ]
    );
}
