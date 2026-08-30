# LogikSmith guide

LogikSmith is a lightweight KNX automation engine with a portable Rust core. It runs on a desktop host first and may later run on OpenKNX-compatible hardware.

## Working rules

- Work on the active milestone only. Record later ideas in `docs/deferred.md` when that file exists.
- Agree the milestone scope, architecture, and acceptance scenario before implementation.
- Keep platform, transport, and operating-system concerns out of `logiksmith-core`.
- Extend existing concepts and remove superseded temporary fields instead of creating two sources of truth.
- Keep proof-of-concept code small. Add abstractions only when the active milestone needs them.
- The lead agent owns architecture, integration, and the complete KNX event-to-actuator path.
- Subagents may handle isolated tests, research, review, or leaf components when their interface and owner are explicit.
- Automated tests must never access the physical KNX installation. Hardware verification is a deliberate manual acceptance step.
- Use `mise`, `./scripts/bootstrap.sh`, and `./scripts/run-dev.sh` for the desktop POC. Keep `config/local.toml` ignored.
- Before a manual KNX run, confirm the output is a harmless visible actuator, the input and output addresses differ, and the gateway uses plain KNXnet/IP tunnelling.
- A successful manual POC run shows `KNX connected`, then an input `true`, an immediate output `true`, and an output `false` after the configured delay. Test retriggering and shutdown deliberately.
- Keep every tracked source file under 1000 lines by using focussed modules for cohesive responsibilities.
- Serialize every revision exposed through JSON, including optional nested revisions, as a non-negative decimal string. Add a JSON-shape assertion whenever a new revision field is introduced; the browser intentionally rejects numeric revisions to avoid JavaScript precision loss.

Start with the active PRD, then consult `docs/architecture.md`, `docs/decisions.md`, `docs/deferred.md`, and `docs/issues/` as those records are added.
