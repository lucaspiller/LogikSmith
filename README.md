# LogikSmith

![](docs/media/logo.png)

A lightweight KNX automation engine. The current proof of concept validates a
portable Rust core on a desktop host, with KNXnet/IP transport provided by a
Python/XKNX sidecar.

## POC path

The desktop host owns the event loop and hard-coded five-second behavior. The
core receives typed DPT 1.001 events and emits typed writes; the sidecar only
translates those writes to and from KNX. They communicate over versioned NDJSON
on the sidecar's stdin/stdout. See [the bridge wire contract](docs/bridge-protocol.md).

## Bootstrap and run

The scripts use the pinned toolchains managed by `mise` and a standard-library
Python virtual environment:

```sh
./scripts/bootstrap.sh
cp config/local.toml.example config/local.toml
# Edit config/local.toml with the intended KNX interface and group addresses.
./scripts/run-dev.sh
```

Never commit `config/local.toml`. The templates use a reserved documentation IP
and invalid address placeholders, so they must be edited before the host can
connect. Confirm that the output group address controls only a harmless,
visible test actuator (never a lock, alarm, heating, mains, or other
safety-critical load), and that it differs from the input address.

## Test boundary

Run automated tests with:

```sh
cargo test --workspace
```

They use deterministic core APIs, protocol fixtures, and/or a fake bridge.
They must never connect to a physical KNX installation. The real KNX check is
deliberately manual.

## Manual KNX acceptance

With a verified plain KNXnet/IP tunnelling gateway and safe DPT 1.001 input
and output configured:

1. Start `./scripts/run-dev.sh` and confirm logs show `KNX connected` and
   `LogikSmith ready`.
2. Trigger the input with `true`; confirm the output turns on.
3. Confirm the output turns off about five seconds later.
4. Trigger `true` again before the deadline; confirm the off deadline extends.
5. Confirm `false` and unrelated group addresses do not trigger the output.
6. Stop with Ctrl+C and confirm the bridge shuts down cleanly.
