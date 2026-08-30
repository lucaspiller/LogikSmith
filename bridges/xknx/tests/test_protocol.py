from types import SimpleNamespace
import unittest

from logiksmith_xknx.bridge import bool_from_xknx, event_from_telegram, percent_from_xknx, temperature_from_xknx
from logiksmith_xknx.protocol import (
    DPT_1_001,
    DPT_5_001,
    DPT_9_001,
    BoolValue,
    BridgeHello,
    CommandResult,
    Configure,
    Dpt,
    Fatal,
    GroupAddressDpt,
    KnxEvent,
    KnxWrite,
    PercentValue,
    TemperatureValue,
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
            GroupAddressDpt("2/3/52", DPT_5_001),
            GroupAddressDpt("2/4/52", DPT_1_001),
        ),
    )


class ProtocolRoundTripTests(unittest.TestCase):
    def test_all_v1_messages_round_trip(self) -> None:
        messages = (
            BridgeHello("xknx", "0.1.0", "3.19.0"),
            configure(),
            Ready("knxip_tunneling", "192.0.2.20"),
            KnxEvent("1.1.42", "2/2/52", "group_value_write", DPT_1_001, BoolValue("bool", True)),
            KnxEvent("1.1.42", "2/3/52", "group_value_response", DPT_5_001, PercentValue("percent", 42)),
            KnxWrite(12, "2/3/52", DPT_5_001, PercentValue("percent", 100)),
            KnxWrite(13, "2/4/52", DPT_9_001, TemperatureValue("temperature", -4.25)),
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

    def test_config_requires_supported_nonempty_unique_addresses(self) -> None:
        with self.assertRaises(ProtocolError):
            Configure(
                TunnelingConnection("192.0.2.20", 3671, None),
                (GroupAddressDpt("2/2/52", Dpt(1, 2)),),
            )
        with self.assertRaises(ProtocolError):
            parse_line(
                '{"v":1,"type":"configure","connection":{"type":"tunneling","gateway_ip":"192.0.2.20","gateway_port":3671,"local_ip":null},"group_addresses":[{"address":"2/2/52","dpt":{"major":1,"subtype":1}},{"address":"2/2/52","dpt":{"major":1,"subtype":1}}]}'
            )
        with self.assertRaises(ProtocolError):
            Configure(TunnelingConnection("192.0.2.20", 3671, None), ())

    def test_config_accepts_one_address_and_mixed_supported_dpts(self) -> None:
        parsed = parse_line(
            '{"v":1,"type":"configure","connection":{"type":"tunneling","gateway_ip":"192.0.2.20","gateway_port":3671,"local_ip":null},"group_addresses":[{"address":"2/3/52","dpt":{"major":5,"subtype":1}}]}'
        )
        self.assertEqual(parsed.group_addresses, (GroupAddressDpt("2/3/52", DPT_5_001),))

    def test_config_accepts_temperature_dpt(self) -> None:
        parsed = parse_line(
            '{"v":1,"type":"configure","connection":{"type":"tunneling","gateway_ip":"192.0.2.20","gateway_port":3671,"local_ip":null},"group_addresses":[{"address":"2/4/52","dpt":{"major":9,"subtype":1}}]}'
        )
        self.assertEqual(parsed.group_addresses, (GroupAddressDpt("2/4/52", DPT_9_001),))

    def test_temperature_value_rejects_non_finite(self) -> None:
        with self.assertRaises(ProtocolError):
            TemperatureValue.from_obj({"kind": "temperature", "value": float("nan")})

    def test_percent_value_validation_and_dpt_mismatch(self) -> None:
        self.assertEqual(PercentValue.from_obj({"kind": "percent", "value": 0}).value, 0)
        self.assertEqual(PercentValue.from_obj({"kind": "percent", "value": 100}).value, 100)
        for value in (-1, 101, 42.5, True):
            with self.assertRaises(ProtocolError):
                PercentValue.from_obj({"kind": "percent", "value": value})
        with self.assertRaises(ProtocolError):
            parse_line(
                '{"v":1,"type":"knx_write","request_id":1,"destination":"2/3/52","dpt":{"major":5,"subtype":1},"value":{"kind":"bool","value":true}}'
            )
        with self.assertRaises(ProtocolError):
            parse_line(
                '{"v":1,"type":"knx_write","request_id":1,"destination":"2/3/52","dpt":{"major":1,"subtype":1},"value":{"kind":"percent","value":42}}'
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

    def test_percent_conversion_accepts_integer_boundaries_only(self) -> None:
        self.assertEqual(percent_from_xknx(0), 0)
        self.assertEqual(percent_from_xknx(100), 100)
        self.assertEqual(percent_from_xknx(SimpleNamespace(value=42)), 42)
        for value in (-1, 101, 42.5, True, "42"):
            with self.assertRaises(ValueError):
                percent_from_xknx(value)

    def test_temperature_conversion_accepts_finite_numbers_only(self) -> None:
        self.assertEqual(temperature_from_xknx(-4.25), -4.25)
        self.assertEqual(temperature_from_xknx(SimpleNamespace(value=21.5)), 21.5)
        for value in (True, float("nan"), float("inf"), "21.5"):
            with self.assertRaises(ValueError):
                temperature_from_xknx(value)

    def test_telegram_mapper_preserves_service(self) -> None:
        class GroupValueWrite:
            pass

        class GroupValueResponse:
            pass

        class GroupValueRead:
            pass

        def telegram(
            payload: object,
            value: object = None,
            destination: str = "2/2/52",
        ) -> object:
            return SimpleNamespace(
                source_address="1.1.42",
                destination_address=destination,
                payload=payload,
                decoded_data=None if value is None else SimpleNamespace(value=value),
            )

        configured = {
            "2/2/52": DPT_1_001,
            "2/3/52": DPT_5_001,
        }
        write = event_from_telegram(telegram(GroupValueWrite(), True), configured)
        response = event_from_telegram(telegram(GroupValueResponse(), False), configured)
        read = event_from_telegram(telegram(GroupValueRead()), configured)
        self.assertEqual(write.service, "group_value_write")
        self.assertTrue(write.value.value)
        self.assertEqual(response.service, "group_value_response")
        self.assertFalse(response.value.value)
        self.assertEqual(read.service, "group_value_read")
        self.assertIsNone(read.value)
        self.assertIsNone(
            event_from_telegram(telegram(GroupValueWrite(), True, "2/4/52"), configured)
        )

    def test_telegram_mapper_decodes_configured_percent_and_observes_output(self) -> None:
        class GroupValueWrite:
            pass

        telegram = SimpleNamespace(
            source_address="1.1.42",
            destination_address="2/3/52",
            payload=GroupValueWrite(),
            decoded_data=SimpleNamespace(value=42),
        )
        event = event_from_telegram(
            telegram,
            (
                GroupAddressDpt("2/2/52", DPT_1_001),
                GroupAddressDpt("2/3/52", DPT_5_001),
            ),
        )
        self.assertEqual(event.destination, "2/3/52")
        self.assertEqual(event.dpt, DPT_5_001)
        self.assertEqual(event.value, PercentValue("percent", 42))

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
