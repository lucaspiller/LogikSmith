# LogikSmith design brief

LogikSmith is a programmable automation runtime for KNX buildings. It gives integrators a clear place to express behaviour in code, while KNX remains the building bus that carries values to and from the installation. The product principle is simple: logic should be portable, typed, observable, replayable, and safe to run on both desktop and embedded hosts.

The milestone plan describes the order of delivery. This brief describes the destination: a focused logic appliance with an excellent authoring and debugging loop, rather than a loose collection of integrations. The desktop KNX proof of concept has validated the first architectural seam, and the rest of the product grows through that seam.

## Product shape

People should work with named, typed logic blocks. A block describes a piece of building behaviour in terms that make sense to the building, then bindings connect those names to KNX group addresses, internal signals, or managed external sources.

```text
Adaptive lighting
  inputs:  presence, lux, night_mode
  outputs: brightness, colour_temperature
  script:  short, bounded decision logic
  state:   private values owned by this block
```

The block is the reusable unit. An integrator can use the same lighting behaviour in another room by changing bindings instead of editing a collection of raw addresses in source code.

## One execution model across every host

The Rust core owns automation semantics. Hosts own time, I/O, storage, lifecycle, and platform limits. This separation allows the same logic model to run against KNX/IP on a laptop and OpenKNX GroupObjects on an ESP32.

```text
                         LogikSmith core
          events · typed values · blocks · timers · state
              effects · validation · execution records
                                 │
                          platform boundary
                   ┌─────────────┴─────────────┐
                   │                           │
             Desktop host                 Embedded host
             macOS / Linux                OpenKNX / ESP32
             KNX/IP                       GroupObjects
             filesystem                   embedded storage
```

The current desktop adapter uses an XKNX sidecar for KNXnet/IP. It translates transport messages at the edge. Decisions, timing, state, and the meaning of an effect stay in Rust.

## Logic blocks act on events

Each block receives one event and a consistent view of its world. It runs a short script, returns effects, and finishes. The next event starts another execution.

```text
event + input snapshot + transient state + persistent values
                              │
                              ▼
                         Lua sandbox
                              │
                              ▼
         outputs + timers + persistence + async requests
```

Scripts return requests for work. The runtime validates those requests and performs the external actions. KNX writes, storage, HTTP calls, schedules, and timer delivery therefore stay visible to the runtime and work the same way on every host.

```lua
return {
    outputs = {
        brightness = 60,
        colour_temperature = 3000,
    },
    timers = {
        off = { after = minutes(5) },
    },
}
```

An output means “send this value for this execution”. Repeating the same output remains valid because another controller may have changed the actuator, a device may have restarted, or an earlier telegram may have been missed.

## Lua stays contained

Lua provides the programmable decision layer. Rust provides containment, typed conversion, validation, and error reporting. A faulty block should fail its own execution while KNX processing and neighbouring blocks continue.

The initial Lua API stays deliberately small:

- event context and input snapshot;
- typed input and output names;
- transient state;
- read access to persistent state;
- declarative effects for outputs, timers, and persistence;
- clear errors, execution limits, and isolated block environments.

Instruction budgets, memory limits where practical, and per-block serial execution are product requirements. They protect the building bus from a bad script and make runtime behaviour explainable.

## Types carry their KNX meaning

KNX datapoint semantics remain attached to values throughout the runtime. A value is its DPT and typed payload, plus the range, units, encoding rules, and validation behaviour that follow from that DPT.

```text
DPT 1.001  → switch boolean
DPT 5.001  → percentage
DPT 9.004  → illuminance
DPT 17.xxx → scene
```

Lua can receive convenient values, and the core retains the DPT identity. That keeps `50` meaningful in a script, a binding, a log entry, a simulator, and an outgoing KNX telegram.

Milestone 2 expands the current DPT 1.001 proof into an initial useful DPT set. DPT support should grow through the shared type model, one family at a time.

## Bindings make logic reusable

Bindings connect logical endpoints to the outside world. They let a script talk about the room or system it controls, not the transport identifier currently wired into ETS.

```text
presence       ← KNX 2/2/52
outside_lux    ← HTTP weather source
night_mode     ← internal signal
brightness     → KNX 2/3/52
audit_event    → webhook sink
```

KNX is the first and primary binding type. Internal signals, configured HTTP sources, webhooks, and later adapters use the same logical input/output model where it makes sense. The runtime owns retries, timeouts, freshness, authentication, and typed conversion for managed external sources.

## State, timers, and schedules

Blocks need memory, delayed work, and calendar context. Keeping these as explicit runtime concepts produces a compact programming model and avoids long-lived scripts.

| Concept | Purpose | Lifecycle |
| --- | --- | --- |
| Input | Current value supplied by a binding | Read-only during an execution |
| Transient state | Private block state between executions | Cleared on restart |
| Persistent state | Durable per-block values | Updated through explicit effects |
| Named timer | Delayed or retriggered work | Delivers a later timer event |
| Schedule | Fixed, repeating, or astronomical time rules | Delivers a later schedule event |

Named timers cover common building behaviour such as delayed off, staircase lighting, watchdogs, short and long press handling, and double press windows. Reusing a timer name resets its deadline naturally.

Schedules eventually cover clock time, intervals, weekday rules, and sunrise or sunset offsets. Scripts receive high-level time events; platform clock APIs remain on the host side of the boundary.

## Many blocks, loosely coupled

LogikSmith grows from one block to many independently configured blocks. Each block owns its script environment, state, timers, limits, execution history, and error state. The runtime serialises events per block so a block never races itself.

Typed internal signals give blocks a clean way to cooperate. A signal has the same useful properties as an external value: a type, a current value, an event when it changes, and visible bindings. Blocks should remain unaware of each other's implementation details.

## Executions are inspectable and replayable

Observability is part of the runtime model. Every invocation should produce an execution record that explains the whole decision path.

```text
Trigger: presence false → true
Inputs:  presence=true, lux=42, night_mode=false
State:   occupied=false
Persistent: activation_count=174
Effects: brightness=60, timer off in 5 minutes
Result:  KNX write accepted, timer scheduled
```

The record includes the trigger, input snapshot, state before and after, persistent snapshot, declared effects, errors, logs, and execution duration. That record feeds terminal logs today, simulation and management APIs later, and the browser interface once the runtime model has enough substance to inspect.

Simulation executes the same script and validation path with external effects suppressed. Captured executions should replay from the event, inputs, state, and persistent values. This discipline keeps debugging useful when a physical building is inconvenient or unsafe to reproduce.

## The management experience

The browser interface is a core part of how the product earns trust. It follows the execution, tracing, and simulation foundations so it can show reality rather than a second hand-made interpretation of runtime state.

The first meaningful management experience covers:

- logic block source, validation errors, and enablement;
- named inputs, outputs, types, and bindings;
- current values and value validity;
- last execution, errors, returned effects, and logs;
- safe simulation with supplied or captured inputs;
- script editing, validation, and activation without reflashing firmware.

The UI need not become a full IDE. It needs to make the edit, inspect, simulate, and activate loop materially better than maintaining complex logic graphs in ETS.

## Safety and operations

Building logic must remain predictable when scripts, devices, networks, and storage fail. The runtime needs clear limits for blocks, timers, queued events, pending requests, persistent values, execution time, memory, and diagnostic history.

The host should detect feedback loops, repeated failures, malformed configuration, unavailable external services, and resource pressure. It should surface the cause, preserve KNX responsiveness, and allow a block to be disabled safely. Production hardening comes before treating the desktop host as a continuously running controller.

## Platform direction

Desktop is the primary development environment. It supports rapid iteration against a real KNX installation through KNX/IP and gives the runtime a place to develop complete tests, simulation, and diagnostics before embedded constraints enter the conversation.

The embedded host swaps platform services while preserving runtime semantics:

```text
Desktop:  KNX/IP, native filesystem, desktop networking
Embedded: OpenKNX GroupObjects, LittleFS, ESP networking
```

The OpenKNX host owns ETS-derived configuration, device lifecycle, KNX bus interaction, and platform storage. The Rust core owns logic execution and scripting. The embedded target needs explicit memory budgets, bounded queues, safe script activation, storage recovery, and behaviour checks against the same desktop scenarios.

## How to judge new features

Use this question when a feature proposal arrives:

> Can an execution be replayed from its event, input snapshot, state, and persistent snapshot without touching the outside world?

A positive answer usually points to a good fit. Ongoing I/O and long-lived work should become a managed source, a validated effect, or a later event.

The point is practical: predictable behaviour, useful simulation, compact embedded code, and diagnostics that still make sense at 2am.

## Product position

Gira X1/L1 shows the breadth of automation a building controller can need. LogicMachine shows the flexibility scripting brings to integrators. LogikSmith should concentrate on readable code, typed bindings, declarative effects, live inspection, safe simulation, desktop development, and eventual embedded deployment.

It earns its place by making building logic easier to understand, safer to change, and practical to run where the building needs it.
