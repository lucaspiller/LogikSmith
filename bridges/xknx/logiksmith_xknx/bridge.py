"""Async XKNX sidecar process.

There is intentionally no automation logic here.  The process owns only the
KNX connection and translates between XKNX telegrams and the typed protocol.
"""

from __future__ import annotations

import asyncio
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
import logging
import math
import sys
from typing import Any, Callable, TextIO

from .protocol import (
    BRIDGE_VERSION,
    DPT_1_001,
    DPT_5_001,
    DPT_9_001,
    BridgeHello,
    BoolValue,
    CommandResult,
    Configure,
    Dpt,
    Fatal,
    GroupAddressDpt,
    KnxEvent,
    KnxWrite,
    Message,
    PercentValue,
    TemperatureValue,
    ProtocolError,
    Ready,
    Shutdown,
    encode_message,
    parse_line,
    validate_group_address,
)


LOGGER = logging.getLogger("logiksmith_xknx")


class BridgeError(RuntimeError):
    """A bridge setup or runtime error that should become a fatal message."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


@dataclass(frozen=True, slots=True)
class XknxRuntime:
    """Connected XKNX object and its public write helper."""

    xknx: Any
    group_value_write: Callable[..., None]


def bool_from_xknx(value: Any) -> bool:
    """Convert only a DPT 1.001-shaped XKNX value to a Python bool.

    XKNX 3.19 decodes DPT 1.001 as the ``Switch`` enum, while a raw
    ``DPTBinary`` payload exposes an integer.  Both are accepted; every other
    value is rejected so unsupported/malformed telegrams cannot trigger Rust.
    """
    if type(value) is bool:
        return value
    if type(value) is int and value in (0, 1):
        return bool(value)
    enum_value = getattr(value, "value", None)
    if type(enum_value) is bool:
        return enum_value
    raise ValueError(f"unsupported DPT 1.001 value: {value!r}")


def percent_from_xknx(value: Any) -> int:
    """Convert a DPT 5.001 value to its protocol percentage integer."""
    if type(value) is int and 0 <= value <= 100:
        return value
    enum_value = getattr(value, "value", None)
    if type(enum_value) is int and not isinstance(enum_value, bool) and 0 <= enum_value <= 100:
        return enum_value
    raise ValueError(f"unsupported DPT 5.001 value: {value!r}")


def temperature_from_xknx(value: Any) -> float:
    """Convert a DPT 9.001 value to finite degrees Celsius."""
    candidate = value
    if type(candidate) not in (int, float):
        candidate = getattr(value, "value", None)
    if type(candidate) not in (int, float) or not math.isfinite(candidate):
        raise ValueError(f"unsupported DPT 9.001 value: {value!r}")
    return float(candidate)


def value_from_xknx(value: Any, dpt: Dpt) -> BoolValue | PercentValue | TemperatureValue:
    if dpt == DPT_1_001:
        return BoolValue("bool", bool_from_xknx(value))
    if dpt == DPT_5_001:
        return PercentValue("percent", percent_from_xknx(value))
    if dpt == DPT_9_001:
        return TemperatureValue("temperature", temperature_from_xknx(value))
    raise ValueError(f"unsupported configured DPT: {dpt}")


def _configured_dpt_map(
    group_addresses: Mapping[str, Dpt] | Iterable[GroupAddressDpt] | str,
) -> dict[str, Dpt]:
    """Normalize configured addresses for the telegram mapper.

    The string form is retained for callers of the old one-address helper;
    bridge runtime code always passes the complete configured map.
    """
    if isinstance(group_addresses, str):
        return {validate_group_address(group_addresses, "group address"): DPT_1_001}
    if isinstance(group_addresses, Mapping):
        entries = group_addresses.items()
    else:
        entries = ((entry.address, entry.dpt) for entry in group_addresses)
    result: dict[str, Dpt] = {}
    for address, dpt in entries:
        canonical = validate_group_address(address, "group address")
        if canonical in result:
            raise ProtocolError(f"duplicate configured group address: {canonical}")
        if not isinstance(dpt, Dpt):
            raise ProtocolError(f"DPT for {canonical} must be a DPT record")
        if dpt not in (DPT_1_001, DPT_5_001, DPT_9_001):
            raise ProtocolError(f"unsupported configured DPT for {canonical}: {dpt}")
        result[canonical] = dpt
    if not result:
        raise ProtocolError("group_addresses must not be empty")
    return result


def event_from_telegram(
    telegram: Any,
    group_addresses: Mapping[str, Dpt] | Iterable[GroupAddressDpt] | str,
) -> KnxEvent | None:
    """Map one configured XKNX telegram to a typed event, without XKNX imports.

    ``TelegramQueue`` filters the callback to all configured addresses.  The
    explicit address check remains here as a second correctness boundary. The
    mapper keys on public XKNX APCI class names, keeping tests free of gateway
    objects while retaining write/response/read distinctions.
    """
    configured = _configured_dpt_map(group_addresses)
    destination = validate_group_address(str(telegram.destination_address), "destination")
    dpt = configured.get(destination)
    if dpt is None:
        return None

    payload = getattr(telegram, "payload", None)
    payload_name = type(payload).__name__ if payload is not None else ""
    service_by_payload = {
        "GroupValueWrite": "group_value_write",
        "GroupValueResponse": "group_value_response",
        "GroupValueRead": "group_value_read",
    }
    service = service_by_payload.get(payload_name)
    if service is None:
        return None

    event_value = None
    if service != "group_value_read":
        decoded = getattr(telegram, "decoded_data", None)
        candidate = getattr(decoded, "value", None) if decoded is not None else None
        if candidate is None:
            candidate = getattr(payload, "value", None)
        event_value = value_from_xknx(candidate, dpt)

    source = getattr(telegram, "source_address", None)
    source_string = None if source is None else str(source)
    return KnxEvent(
        source=source_string,
        destination=destination,
        service=service,
        dpt=dpt,
        value=event_value,
    )


class ProtocolWriter:
    """Write one protocol message per line and nothing else to stdout."""

    def __init__(self, stream: TextIO) -> None:
        self.stream = stream
        self.fatal_sent = False

    def send(self, message: Message) -> None:
        self.stream.write(encode_message(message) + "\n")
        self.stream.flush()

    def fatal(self, code: str, message: str) -> None:
        if self.fatal_sent:
            return
        self.fatal_sent = True
        try:
            self.send(Fatal(code, message))
        except (BrokenPipeError, OSError):
            LOGGER.exception("[XKNX] unable to write fatal protocol message")


async def _read_line(stream: TextIO) -> str:
    """Read a blocking text pipe without blocking XKNX's asyncio loop."""
    return await asyncio.to_thread(stream.readline)


async def _read_message(stream: TextIO) -> Message | None:
    line = await _read_line(stream)
    if line == "":
        return None
    return parse_line(line)


def _xknx_version() -> str:
    try:
        from xknx.__version__ import __version__
    except ImportError:
        return "unavailable"
    return __version__


async def _make_xknx(
    config: Configure,
    writer: ProtocolWriter,
    state_queue: asyncio.Queue[Any],
) -> XknxRuntime:
    """Create and connect a non-secure, non-reconnecting XKNX tunnel."""
    try:
        from xknx import XKNX
        from xknx.io import ConnectionConfig as XknxConnectionConfig
        from xknx.io import ConnectionType
        from xknx.telegram import GroupAddress
        from xknx.tools import group_value_write
    except ImportError as exc:
        raise BridgeError("xknx_unavailable", f"XKNX import failed: {exc}") from exc

    connection = XknxConnectionConfig(
        connection_type=ConnectionType.TUNNELING,
        gateway_ip=config.connection.gateway_ip,
        gateway_port=config.connection.gateway_port,
        local_ip=config.connection.local_ip,
        auto_reconnect=False,
    )

    def connection_state_changed(state: Any) -> None:
        state_queue.put_nowait(state)
        LOGGER.debug("[XKNX] connection state=%s", getattr(state, "value", state))

    configured_dpts = {entry.address: entry.dpt for entry in config.group_addresses}

    def telegram_received(telegram: Any) -> None:
        try:
            event = event_from_telegram(telegram, configured_dpts)
        except (ProtocolError, ValueError) as exc:
            LOGGER.warning("[XKNX] ignoring malformed/unsupported telegram: %s", exc)
            return
        if event is None:
            return
        LOGGER.debug(
            "[XKNX] RX %s -> %s %s value=%s",
            event.source,
            event.destination,
            event.service,
            None if event.value is None else event.value.value,
        )
        try:
            writer.send(event)
        except (BrokenPipeError, OSError):
            LOGGER.exception("[XKNX] Rust host stdout closed")

    xknx: Any | None = None
    try:
        xknx = XKNX(connection_config=connection)
        xknx.group_address_dpt.set({address: str(dpt) for address, dpt in configured_dpts.items()})
        xknx.telegram_queue.register_telegram_received_cb(
            telegram_received,
            group_addresses=[GroupAddress(entry.address) for entry in config.group_addresses],
        )
        xknx.connection_manager.register_connection_state_changed_cb(connection_state_changed)
        await xknx.start()
    except BridgeError:
        raise
    except Exception as exc:
        LOGGER.exception(
            "[XKNX] unable to establish non-secure tunnel gateway=%s",
            config.connection.gateway_ip,
        )
        if xknx is not None:
            await _stop_xknx(xknx)
        raise BridgeError("knx_connection_failed", str(exc)) from exc
    assert xknx is not None
    if not xknx.connection_manager.connected.is_set():
        await _stop_xknx(xknx)
        raise BridgeError("knx_connection_failed", "XKNX start returned before tunnel connected")
    return XknxRuntime(xknx, group_value_write)


async def _stop_xknx(xknx: Any) -> None:
    try:
        async with asyncio.timeout(3):
            await xknx.stop()
    except TimeoutError:
        LOGGER.error("[XKNX] timeout while disconnecting")
    except Exception:
        LOGGER.exception("[XKNX] error while disconnecting")


async def _serve(
    runtime: XknxRuntime,
    writer: ProtocolWriter,
    stdin: TextIO,
    state_queue: asyncio.Queue[Any],
) -> int:
    read_task = asyncio.create_task(_read_message(stdin))
    state_task = asyncio.create_task(state_queue.get())
    try:
        while True:
            done, _ = await asyncio.wait(
                (read_task, state_task), return_when=asyncio.FIRST_COMPLETED
            )
            if state_task in done:
                state = state_task.result()
                state_name = getattr(state, "name", str(state))
                if state_name == "DISCONNECTED":
                    writer.fatal("knx_connection_lost", "established KNX/IP tunnel was lost")
                    return 1
                state_task = asyncio.create_task(state_queue.get())

            if read_task in done:
                message = read_task.result()
                if message is None:
                    LOGGER.error("[XKNX] Rust host closed stdin unexpectedly")
                    writer.fatal("bridge_stdin_closed", "Rust host closed stdin unexpectedly")
                    return 1
                if isinstance(message, Shutdown):
                    LOGGER.info("[XKNX] shutdown requested")
                    return 0
                if not isinstance(message, KnxWrite):
                    raise ProtocolError(
                        f"unexpected message after ready: {message.type}"
                    )
                try:
                    runtime.group_value_write(
                        runtime.xknx,
                        message.destination,
                        message.value.value,
                        value_type=str(message.dpt),
                    )
                except Exception as exc:
                    LOGGER.exception(
                        "[XKNX] TX failed request_id=%s destination=%s",
                        message.request_id,
                        message.destination,
                    )
                    writer.send(CommandResult(message.request_id, False, str(exc)))
                else:
                    LOGGER.info(
                        "[XKNX] TX request_id=%s destination=%s dpt=%s value=%s",
                        message.request_id,
                        message.destination,
                        message.dpt,
                        message.value.value,
                    )
                    writer.send(CommandResult(message.request_id, True))
                read_task = asyncio.create_task(_read_message(stdin))
    finally:
        for task in (read_task, state_task):
            if not task.done():
                task.cancel()
        await asyncio.gather(read_task, state_task, return_exceptions=True)


async def run_bridge(
    stdin: TextIO = sys.stdin,
    stdout: TextIO = sys.stdout,
) -> int:
    """Run the bridge protocol; return a process exit status."""
    writer = ProtocolWriter(stdout)
    writer.send(BridgeHello("xknx", BRIDGE_VERSION, _xknx_version()))
    runtime: XknxRuntime | None = None
    try:
        first = await _read_message(stdin)
        if not isinstance(first, Configure):
            if first is None:
                raise ProtocolError("expected configure, got end of input")
            raise ProtocolError(f"expected configure, got {first.type}")

        state_queue: asyncio.Queue[Any] = asyncio.Queue()
        runtime = await _make_xknx(first, writer, state_queue)
        writer.send(Ready("knxip_tunneling", first.connection.gateway_ip))
        return await _serve(runtime, writer, stdin, state_queue)
    except BridgeError as exc:
        LOGGER.error("[XKNX] %s: %s", exc.code, exc)
        writer.fatal(exc.code, str(exc))
        return 1
    except ProtocolError as exc:
        LOGGER.error("[XKNX] protocol error: %s", exc)
        writer.fatal("protocol_error", str(exc))
        return 1
    except (BrokenPipeError, OSError):
        LOGGER.exception("[XKNX] protocol output failed")
        return 1
    except Exception as exc:
        LOGGER.exception("[XKNX] unhandled bridge failure")
        writer.fatal("bridge_failure", str(exc))
        return 1
    finally:
        if runtime is not None:
            await _stop_xknx(runtime.xknx)


def configure_logging() -> None:
    logging.basicConfig(
        level=logging.INFO,
        stream=sys.stderr,
        format="[XKNX] %(levelname)s %(message)s",
    )
    logging.getLogger("xknx").setLevel(logging.INFO)


def main() -> int:
    configure_logging()
    try:
        return asyncio.run(run_bridge())
    except KeyboardInterrupt:
        LOGGER.info("[XKNX] interrupted")
        return 0
