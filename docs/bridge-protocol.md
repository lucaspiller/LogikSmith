# Bridge wire protocol

This is the POC companion to PRD sections 15 to 19. It is the single wire
contract for `logiksmith-desktop` and the Python/XKNX bridge.

## Transport and invariants

- Use newline-delimited JSON (NDJSON) over the child process stdin/stdout.
- Each line is exactly one UTF-8 JSON object; never pretty-print or split a
  message over multiple lines.
- Every object has integer `v: 1` and a string `type`.
- Bridge stdout is protocol-only. Logs, warnings, tracebacks, and diagnostics
  go to stderr. The desktop may capture stderr into its bridge logger.
- DPT 1.001 is represented as `{ "major": 1, "subtype": 1 }`; its value is
  `{ "kind": "bool", "value": true|false }`.
- DPT 5.001 is represented as `{ "major": 5, "subtype": 1 }`; its value is
  `{ "kind": "percent", "value": 0..100 }`. The percentage value is an
  integer, including the boundary values 0 and 100.
- The bridge supports only DPT 1.001 and DPT 5.001. Values whose `kind` does
  not match their DPT, malformed values, and out-of-range percentages are
  rejected.
- Three-level group addresses use main `0..31`, middle `0..7`, and subgroup
  `0..255`. The reserved broadcast address `0/0/0` is rejected here. These
  rules follow the [KNX Association group-address properties](https://support.knx.org/hc/en-us/articles/360022048059-Group-Address-panel-detailed)
  and [reserved ranges](https://support.knx.org/hc/en-us/articles/115001825304-Group-Address-Ranges).

## Startup sequence

```text
desktop spawns bridge
        ↓
bridge_hello
        ↓
desktop validates v=1 and sends configure
        ↓
bridge configures XKNX and establishes the KNXnet/IP tunnel
        ↓
ready
        ↓
desktop starts normal event/timer processing
```

The bridge must not send `ready` until the tunnel is established. The
connection is plain KNXnet/IP tunnelling; secure tunnelling is out of scope.

## Messages

The following shapes are exact for this POC. Field values below mirror the
PRD examples; the configured values are supplied by the desktop.

Bridge → desktop: hello

```json
{"v":1,"type":"bridge_hello","bridge":"xknx","bridge_version":"0.1.0","xknx_version":"..."}
```

Desktop → bridge: configure

```json
{"v":1,"type":"configure","connection":{"type":"tunneling","gateway_ip":"192.0.2.20","gateway_port":3671,"local_ip":null},"group_addresses":[{"address":"2/2/52","dpt":{"major":1,"subtype":1}},{"address":"2/3/52","dpt":{"major":5,"subtype":1}},{"address":"2/4/52","dpt":{"major":1,"subtype":1}}]}
```

`local_ip` is either a string or `null`. `group_addresses` is a non-empty list
of unique address/DPT records. Every configured address is observed; there is
no input/output ordering or naming in the bridge. The desktop decides whether
an observed event drives logic.

Bridge → desktop: ready

```json
{"v":1,"type":"ready","transport":"knxip_tunneling","gateway":"192.0.2.20"}
```

Bridge → desktop: incoming KNX event

```json
{"v":1,"type":"knx_event","source":"1.1.42","destination":"2/2/52","service":"group_value_write","dpt":{"major":1,"subtype":1},"value":{"kind":"bool","value":true}}
```

For a DPT 5.001 telegram, the event carries an integer percentage:

```json
{"v":1,"type":"knx_event","source":"1.1.42","destination":"2/3/52","service":"group_value_write","dpt":{"major":5,"subtype":1},"value":{"kind":"percent","value":42}}
```

Desktop → bridge: KNX write

```json
{"v":1,"type":"knx_write","request_id":12,"destination":"2/4/52","dpt":{"major":1,"subtype":1},"value":{"kind":"bool","value":true}}
```

The write `dpt` and `value.kind` must agree. For DPT 5.001, use an integer
percentage from 0 through 100:

```json
{"v":1,"type":"knx_write","request_id":13,"destination":"2/3/52","dpt":{"major":5,"subtype":1},"value":{"kind":"percent","value":42}}
```

Bridge → desktop: write result

```json
{"v":1,"type":"command_result","request_id":12,"ok":true}
```

For a write failure, set `ok` to `false` and include `error`:

```json
{"v":1,"type":"command_result","request_id":12,"ok":false,"error":"KNX connection unavailable"}
```

Bridge → desktop: terminal failure

```json
{"v":1,"type":"fatal","code":"knx_connection_failed","message":"Unable to establish KNX/IP tunnel"}
```

Desktop → bridge: shutdown

```json
{"v":1,"type":"shutdown"}
```

## Failure behaviour

Protocol-version mismatch, malformed/non-JSON stdout, an invalid startup
message, or an unrecognized message terminates the session. When the bridge
detects a terminal failure and can still write protocol output, it emits one
`fatal` message, then exits non-zero. When the desktop detects malformed
output or a protocol mismatch, it logs the protocol error and exits non-zero.
A bridge that cannot start is a desktop startup error and has no protocol
message.

Failure to establish the tunnel, or loss of an established tunnel, is fatal;
do not reconnect in this POC. An individual KNX write failure is reported by
`command_result` with `ok: false` and is non-fatal while the bridge remains
connected. The desktop must treat bridge `fatal` or unexpected exit as
terminal and must not silently continue.
