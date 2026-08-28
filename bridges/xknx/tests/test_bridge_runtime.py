import asyncio
import io
from types import ModuleType, SimpleNamespace
import sys
from unittest.mock import patch
import unittest

from logiksmith_xknx import (
    DPT_1_001,
    DPT_5_001,
    BoolValue,
    Configure,
    GroupAddressDpt,
    KnxWrite,
    PercentValue,
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
    def test_make_xknx_registers_every_configured_address_and_dpt(self) -> None:
        class FakeConnection:
            def __init__(self) -> None:
                self.connected = asyncio.Event()

            def register_connection_state_changed_cb(self, callback: object) -> None:
                self.callback = callback

        class FakeTelegramQueue:
            def register_telegram_received_cb(self, callback: object, *, group_addresses: list[object]) -> None:
                self.callback = callback
                self.addresses = group_addresses

        class FakeXKNX:
            instance: "FakeXKNX | None" = None

            def __init__(self, *, connection_config: object) -> None:
                self.connection_config = connection_config
                self.group_address_dpt = SimpleNamespace(set=self.set_dpts)
                self.telegram_queue = FakeTelegramQueue()
                self.connection_manager = FakeConnection()
                self.stopped = False
                FakeXKNX.instance = self

            def set_dpts(self, dpts: dict[str, str]) -> None:
                self.dpts = dpts

            async def start(self) -> None:
                self.connection_manager.connected.set()

            async def stop(self) -> None:
                self.stopped = True

        class GroupAddress:
            def __init__(self, address: str) -> None:
                self.address = address

            def __str__(self) -> str:
                return self.address

        class GroupValueWrite:
            pass

        def group_value_write(*_args: object, **_kwargs: object) -> None:
            pass

        xknx_module = ModuleType("xknx")
        xknx_module.XKNX = FakeXKNX
        io_module = ModuleType("xknx.io")
        io_module.ConnectionConfig = lambda **kwargs: SimpleNamespace(**kwargs)
        io_module.ConnectionType = SimpleNamespace(TUNNELING="tunneling")
        telegram_module = ModuleType("xknx.telegram")
        telegram_module.GroupAddress = GroupAddress
        tools_module = ModuleType("xknx.tools")
        tools_module.group_value_write = group_value_write
        config = Configure(
            TunnelingConnection("192.0.2.20", 3671, None),
            (
                GroupAddressDpt("2/2/52", DPT_1_001),
                GroupAddressDpt("2/3/52", DPT_5_001),
                GroupAddressDpt("2/4/52", DPT_1_001),
            ),
        )
        output = io.StringIO()
        writer = bridge.ProtocolWriter(output)
        state_queue: asyncio.Queue[object] = asyncio.Queue()
        with patch.dict(
            sys.modules,
            {
                "xknx": xknx_module,
                "xknx.io": io_module,
                "xknx.telegram": telegram_module,
                "xknx.tools": tools_module,
            },
        ):
            runtime = asyncio.run(bridge._make_xknx(config, writer, state_queue))
            fake = FakeXKNX.instance
            self.assertIs(runtime.xknx, fake)
            self.assertEqual(fake.dpts, {"2/2/52": "1.001", "2/3/52": "5.001", "2/4/52": "1.001"})
            self.assertEqual([str(address) for address in fake.telegram_queue.addresses], [
                "2/2/52", "2/3/52", "2/4/52",
            ])
            fake.telegram_queue.callback(SimpleNamespace(
                source_address="1.1.42",
                destination_address=GroupAddress("2/3/52"),
                payload=GroupValueWrite(),
                decoded_data=SimpleNamespace(value=42),
            ))
        messages = [parse_line(line) for line in output.getvalue().splitlines()]
        self.assertEqual(messages[0].destination, "2/3/52")
        self.assertEqual(messages[0].dpt, DPT_5_001)
        self.assertEqual(messages[0].value, PercentValue("percent", 42))
        asyncio.run(bridge._stop_xknx(runtime.xknx))

    def test_handshake_write_result_and_shutdown_without_gateway(self) -> None:
        config = Configure(
            TunnelingConnection("192.0.2.20", 3671, None),
            (
                GroupAddressDpt("2/2/52", DPT_1_001),
                GroupAddressDpt("2/3/52", DPT_5_001),
            ),
        )
        incoming = io.StringIO(
            "\n".join(
                (
                    encode_message(config),
                    encode_message(KnxWrite(7, "2/3/52", DPT_5_001, PercentValue("percent", 42))),
                    encode_message(Shutdown()),
                )
            )
            + "\n"
        )
        outgoing = io.StringIO()
        fake_xknx = FakeXKNX()
        writes: list[tuple[str, int, str]] = []

        def fake_write(_xknx: object, address: str, value: int, **kwargs: object) -> None:
            writes.append((address, value, kwargs["value_type"]))

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
        self.assertEqual(writes, [("2/3/52", 42, "5.001")])
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
