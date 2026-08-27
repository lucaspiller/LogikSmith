import asyncio
import io
from unittest.mock import patch
import unittest

from logiksmith_xknx import (
    DPT_1_001,
    BoolValue,
    Configure,
    GroupAddressDpt,
    KnxWrite,
    Shutdown,
    TunnelingConnection,
    encode_message,
    parse_line,
)
from logiksmith_xknx import bridge


class FakeXKNX:
    def __init__(self) -> None:
        self.stopped = False

    async def stop(self) -> None:
        self.stopped = True


class BridgeRuntimeTests(unittest.TestCase):
    def test_handshake_write_result_and_shutdown_without_gateway(self) -> None:
        config = Configure(
            TunnelingConnection("192.0.2.20", 3671, None),
            (
                GroupAddressDpt("2/2/52", DPT_1_001),
                GroupAddressDpt("2/3/52", DPT_1_001),
            ),
        )
        incoming = io.StringIO(
            "\n".join(
                (
                    encode_message(config),
                    encode_message(KnxWrite(7, "2/3/52", DPT_1_001, BoolValue("bool", True))),
                    encode_message(Shutdown()),
                )
            )
            + "\n"
        )
        outgoing = io.StringIO()
        fake_xknx = FakeXKNX()
        writes: list[tuple[str, bool]] = []

        def fake_write(_xknx: object, address: str, value: bool, **_: object) -> None:
            writes.append((address, value))

        async def fake_make(
            _config: Configure,
            _writer: bridge.ProtocolWriter,
            _state_queue: asyncio.Queue[object],
        ) -> bridge.XknxRuntime:
            return bridge.XknxRuntime(fake_xknx, fake_write)

        with patch.object(bridge, "_make_xknx", fake_make):
            status = asyncio.run(bridge.run_bridge(incoming, outgoing))

        messages = [parse_line(line) for line in outgoing.getvalue().splitlines()]
        self.assertEqual(status, 0)
        self.assertEqual(messages[0].type, "bridge_hello")
        self.assertEqual(messages[1].type, "ready")
        self.assertEqual(messages[2].type, "command_result")
        self.assertTrue(messages[2].ok)
        self.assertEqual(writes, [("2/3/52", True)])
        self.assertTrue(fake_xknx.stopped)

    def test_unexpected_stdin_eof_is_fatal(self) -> None:
        config = Configure(
            TunnelingConnection("192.0.2.20", 3671, None),
            (
                GroupAddressDpt("2/2/52", DPT_1_001),
                GroupAddressDpt("2/3/52", DPT_1_001),
            ),
        )
        incoming = io.StringIO(encode_message(config) + "\n")
        outgoing = io.StringIO()
        fake_xknx = FakeXKNX()

        async def fake_make(
            _config: Configure,
            _writer: bridge.ProtocolWriter,
            _state_queue: asyncio.Queue[object],
        ) -> bridge.XknxRuntime:
            return bridge.XknxRuntime(fake_xknx, lambda *_args, **_kwargs: None)

        with patch.object(bridge, "_make_xknx", fake_make):
            status = asyncio.run(bridge.run_bridge(incoming, outgoing))

        messages = [parse_line(line) for line in outgoing.getvalue().splitlines()]
        self.assertEqual(status, 1)
        self.assertEqual(messages[-1].type, "fatal")
        self.assertEqual(messages[-1].code, "bridge_stdin_closed")
        self.assertTrue(fake_xknx.stopped)


if __name__ == "__main__":
    unittest.main()
