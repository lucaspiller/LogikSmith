# Proof-of-concept implementation handoff

LogikSmith is a KNX automation proof of concept. The goal is a manual physical demonstration: a DPT 1.001 `true` write to the configured input group address makes the configured output turn on, then turn off after five seconds. A later `true` resets that deadline.

The accepted scope and architecture are in `docs/local/proof-of-concept-prd.md`. Treat it as authoritative. The local PRD is intentionally ignored by Git, so do not force-add it. `AGENTS.md` contains the repository rules.

## Decisions already made

- The branch is `proof-of-concept`.
- Build a portable Rust core, Tokio desktop host, and Python XKNX sidecar communicating over typed NDJSON on stdin/stdout.
- The MDT interface uses plain KNXnet/IP tunnelling. KNX IP Secure is out of scope.
- Use mise to pin Rust and Python; use standard-library `venv` for Python dependencies.
- Automated tests must never connect to real KNX hardware. The physical test is manual acceptance only.
- Bridge exit or a lost established tunnel is fatal. Individual write failures are logged but non-fatal while the bridge is connected.
- Keep the code small. The architecture boundary is required; speculative abstractions and speculative file trees are not.

## Current repository state

- Commit `89a4ec3` added a KNX-specific `AGENTS.md`.
- No application code exists yet.
- `docs/local/proof-of-concept-prd.md` was updated locally with approved decisions and is globally ignored.

## Collaboration rules

- Work only in the files assigned in your task. Do not overwrite another agent's area.
- Read `AGENTS.md` and the relevant PRD sections before making changes.
- Use `apply_patch` for edits and run focused verification before reporting back.
- Keep stdout protocol-only in the Python bridge. Python diagnostics go to stderr.
- State every assumption you make in your final handoff, including assumptions later proven correct.
- Do not use real KNX addresses, gateways, or hardware from tests.

## Suggested skills

- `ponytail` for the smallest implementation that preserves the POC contract.
- `write-like-luca` for any documentation changes.
