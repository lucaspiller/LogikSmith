from types import SimpleNamespace
import unittest

from logiksmith_xknx.bridge import bool_from_xknx, event_from_telegram
from logiksmith_xknx.protocol import (
    DPT_1_001,
    BoolValue,
    BridgeHello,
    CommandResult,
    Configure,
    Dpt,
    Fatal,
    GroupAddressDpt,
    KnxEvent,
    KnxWrite,
    ProtocolError,
    Ready,
    Shutdown,
    TunnelingConnection,
    encode_message,
    parse_line,
    validate_group_address,
)


def configure() -> Configure:
    return Configure(
        TunnelingConnection("192.0.2.20", 3671, None),
        (
            GroupAddressDpt("2/2/52", DPT_1_001),
            GroupAddressDpt("2/3/52", DPT_1_001),
        ),
    )


class ProtocolRoundTripTests(unittest.TestCase):
    def test_all_v1_messages_round_trip(self) -> None:
        messages = (
            BridgeHello("xknx", "0.1.0", "3.19.0"),
            configure(),
            Ready("knxip_tunneling", "192.0.2.20"),
            KnxEvent("1.1.42", "2/2/52", "group_value_write", DPT_1_001, BoolValue("bool", True)),
            KnxWrite(12, "2/3/52", DPT_1_001, BoolValue("bool", False)),
            CommandResult(12, True),
            CommandResult(13, False, "KNX connection unavailable"),
            Fatal("knx_connection_failed", "Unable to establish KNX/IP tunnel"),
            Shutdown(),
        )
        for message in messages:
            self.assertEqual(parse_line(encode_message(message)), message)

    def test_protocol_version_is_exact(self) -> None:
        with self.assertRaisesRegex(ProtocolError, "expected=1 received=2"):
            parse_line('{"v":2,"type":"shutdown"}')

    def test_malformed_and_unknown_fields_are_rejected(self) -> None:
        with self.assertRaises(ProtocolError):
            parse_line('{"v":1,"type":"shutdown","extra":true}')
        with self.assertRaises(ProtocolError):
            parse_line('{"v":1,"type":"shutdown","type":"fatal"}')
        with self.assertRaises(ProtocolError):
            parse_line('{"v":1,"type":"not_a_message"}')
        with self.assertRaises(ProtocolError):
            parse_line('{"v":1,"type":"shutdown","x":NaN}')
        with self.assertRaises(ProtocolError):
            parse_line('{"v":1,"type":"command_result","request_id":1,"ok":true,"error":"unexpected"}')

    def test_config_requires_ordered_distinct_dpt_1001_addresses(self) -> None:
        with self.assertRaises(ProtocolError):
            Configure(
                TunnelingConnection("192.0.2.20", 3671, None),
                (GroupAddressDpt("2/2/52", Dpt(1, 2)), GroupAddressDpt("2/3/52", DPT_1_001)),
            )
        with self.assertRaises(ProtocolError):
            parse_line(
                '{"v":1,"type":"configure","connection":{"type":"tunneling","gateway_ip":"192.0.2.20","gateway_port":3671,"local_ip":null},"group_addresses":[{"address":"2/2/52","dpt":{"major":1,"subtype":1}},{"address":"2/2/52","dpt":{"major":1,"subtype":1}}]}'
            )

    def test_group_address_uses_knx_three_level_ranges(self) -> None:
        self.assertEqual(validate_group_address("31/7/255"), "31/7/255")
        for address in ("32/7/255", "31/8/255", "0/0/0"):
            with self.assertRaises(ProtocolError):
                validate_group_address(address)


class ConversionTests(unittest.TestCase):
    def test_bool_conversion_accepts_xknx_shapes_only(self) -> None:
        enum_value = SimpleNamespace(value=True)
        self.assertTrue(bool_from_xknx(True))
        self.assertFalse(bool_from_xknx(0))
        self.assertTrue(bool_from_xknx(enum_value))
        with self.assertRaises(ValueError):
            bool_from_xknx(2)
        with self.assertRaises(ValueError):
            bool_from_xknx((True,))

    def test_telegram_mapper_preserves_service(self) -> None:
        class GroupValueWrite:
            pass

        class GroupValueResponse:
            pass

        class GroupValueRead:
            pass

        def telegram(payload: object, value: object = None) -> object:
            return SimpleNamespace(
                source_address="1.1.42",
                destination_address="2/2/52",
                payload=payload,
                decoded_data=None if value is None else SimpleNamespace(value=value),
            )

        write = event_from_telegram(telegram(GroupValueWrite(), True), "2/2/52")
        response = event_from_telegram(telegram(GroupValueResponse(), False), "2/2/52")
        read = event_from_telegram(telegram(GroupValueRead()), "2/2/52")
        self.assertEqual(write.service, "group_value_write")
        self.assertTrue(write.value.value)
        self.assertEqual(response.service, "group_value_response")
        self.assertFalse(response.value.value)
        self.assertEqual(read.service, "group_value_read")
        self.assertIsNone(read.value)
        self.assertIsNone(event_from_telegram(telegram(GroupValueWrite(), True), "2/3/52"))

    def test_mapper_rejects_non_boolean_payload(self) -> None:
        class GroupValueWrite:
            pass

        telegram = SimpleNamespace(
            source_address="1.1.42",
            destination_address="2/2/52",
            payload=GroupValueWrite(),
            decoded_data=SimpleNamespace(value=2),
        )
        with self.assertRaises(ValueError):
            event_from_telegram(telegram, "2/2/52")


if __name__ == "__main__":
    unittest.main()
