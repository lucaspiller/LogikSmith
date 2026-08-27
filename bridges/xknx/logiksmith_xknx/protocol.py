"""Strict version-one NDJSON protocol used by the XKNX sidecar.

The bridge deliberately keeps this module independent of XKNX.  That makes
the wire contract and the small conversion helpers testable without a KNX
gateway (or even an installed XKNX wheel).
"""

from __future__ import annotations

from dataclasses import dataclass
import ipaddress
import json
from typing import Any, ClassVar, TypeAlias


PROTOCOL_VERSION = 1
BRIDGE_VERSION = "0.1.0"


class ProtocolError(ValueError):
    """A malformed, unsupported, or out-of-order bridge message."""


class ValidationError(ProtocolError):
    """A typed protocol field failed its domain validation."""


def _require_object(value: Any, field: str) -> dict[str, Any]:
    if type(value) is not dict:
        raise ProtocolError(f"{field} must be an object")
    return value


def _require_keys(obj: dict[str, Any], required: set[str], optional: set[str] = set()) -> None:
    unknown = set(obj) - required - optional
    missing = required - set(obj)
    if missing:
        raise ProtocolError(f"missing field(s): {', '.join(sorted(missing))}")
    if unknown:
        raise ProtocolError(f"unknown field(s): {', '.join(sorted(unknown))}")


def _string(value: Any, field: str, *, nonempty: bool = True) -> str:
    if type(value) is not str or (nonempty and not value):
        suffix = " non-empty" if nonempty else ""
        raise ProtocolError(f"{field} must be a{suffix} string")
    return value


def _integer(value: Any, field: str, *, minimum: int | None = None, maximum: int | None = None) -> int:
    if type(value) is not int:
        raise ProtocolError(f"{field} must be an integer")
    if minimum is not None and value < minimum:
        raise ValidationError(f"{field} must be >= {minimum}")
    if maximum is not None and value > maximum:
        raise ValidationError(f"{field} must be <= {maximum}")
    return value


def _boolean(value: Any, field: str) -> bool:
    if type(value) is not bool:
        raise ProtocolError(f"{field} must be a boolean")
    return value


def validate_group_address(value: Any, field: str = "group address") -> str:
    """Validate a three-level KNX address (main 0..31, middle 0..7, sub 0..255)."""
    address = _string(value, field)
    parts = address.split("/")
    if len(parts) != 3 or any(not part.isdigit() for part in parts):
        raise ValidationError(f"{field} must use main/middle/subgroup form")
    if any(len(part) > 1 and part.startswith("0") for part in parts):
        raise ValidationError(f"{field} must not contain leading zeroes")
    main, middle, subgroup = (int(part) for part in parts)
    if not 0 <= main <= 31:
        raise ValidationError(f"{field} main group must be in range 0..31")
    if not 0 <= middle <= 7:
        raise ValidationError(f"{field} middle group must be in range 0..7")
    if not 0 <= subgroup <= 255:
        raise ValidationError(f"{field} subgroup must be in range 0..255")
    if main == middle == subgroup == 0:
        raise ValidationError(f"{field} broadcast address 0/0/0 is not valid here")
    return f"{main}/{middle}/{subgroup}"


def validate_individual_address(value: Any, field: str = "source") -> str:
    """Validate and canonicalize a KNX individual address."""
    address = _string(value, field)
    parts = address.split(".")
    if len(parts) != 3 or any(not part.isdigit() for part in parts):
        raise ValidationError(f"{field} must use area.line.device form")
    if any(len(part) > 1 and part.startswith("0") for part in parts):
        raise ValidationError(f"{field} must not contain leading zeroes")
    area, line, device = (int(part) for part in parts)
    if not 0 <= area <= 15 or not 0 <= line <= 15 or not 0 <= device <= 255:
        raise ValidationError(f"{field} components are out of range")
    return f"{area}.{line}.{device}"


def validate_ip(value: Any, field: str = "gateway_ip") -> str:
    address = _string(value, field)
    try:
        ipaddress.ip_address(address)
    except ValueError as exc:
        raise ValidationError(f"{field} must be a valid IP address") from exc
    return address


@dataclass(frozen=True, slots=True)
class Dpt:
    major: int
    subtype: int

    def __post_init__(self) -> None:
        _integer(self.major, "dpt.major", minimum=0, maximum=65535)
        _integer(self.subtype, "dpt.subtype", minimum=0, maximum=65535)

    @classmethod
    def from_obj(cls, value: Any) -> Dpt:
        obj = _require_object(value, "dpt")
        _require_keys(obj, {"major", "subtype"})
        return cls(
            _integer(obj["major"], "dpt.major", minimum=0, maximum=65535),
            _integer(obj["subtype"], "dpt.subtype", minimum=0, maximum=65535),
        )

    def to_obj(self) -> dict[str, int]:
        return {"major": self.major, "subtype": self.subtype}

    def __str__(self) -> str:
        return f"{self.major}.{self.subtype:03d}"


DPT_1_001 = Dpt(1, 1)


@dataclass(frozen=True, slots=True)
class BoolValue:
    kind: str
    value: bool

    def __post_init__(self) -> None:
        if self.kind != "bool":
            raise ValidationError("value.kind must be 'bool'")
        _boolean(self.value, "value.value")

    @classmethod
    def from_obj(cls, value: Any) -> BoolValue:
        obj = _require_object(value, "value")
        _require_keys(obj, {"kind", "value"})
        return cls(
            _string(obj["kind"], "value.kind"),
            _boolean(obj["value"], "value.value"),
        )

    def to_obj(self) -> dict[str, str | bool]:
        return {"kind": self.kind, "value": self.value}


@dataclass(frozen=True, slots=True)
class GroupAddressDpt:
    address: str
    dpt: Dpt

    def __post_init__(self) -> None:
        canonical = validate_group_address(self.address, "group address")
        if canonical != self.address:
            raise ValidationError("group address must be canonical")

    @classmethod
    def from_obj(cls, value: Any) -> GroupAddressDpt:
        obj = _require_object(value, "group_addresses entry")
        _require_keys(obj, {"address", "dpt"})
        return cls(validate_group_address(obj["address"], "group address"), Dpt.from_obj(obj["dpt"]))

    def to_obj(self) -> dict[str, Any]:
        return {"address": self.address, "dpt": self.dpt.to_obj()}


@dataclass(frozen=True, slots=True)
class TunnelingConnection:
    gateway_ip: str
    gateway_port: int
    local_ip: str | None

    def __post_init__(self) -> None:
        if validate_ip(self.gateway_ip) != self.gateway_ip:
            raise ValidationError("gateway_ip must be canonical")
        _integer(self.gateway_port, "gateway_port", minimum=1, maximum=65535)
        if self.local_ip is not None:
            if validate_ip(self.local_ip, "local_ip") != self.local_ip:
                raise ValidationError("local_ip must be canonical")

    @classmethod
    def from_obj(cls, value: Any) -> TunnelingConnection:
        obj = _require_object(value, "connection")
        _require_keys(obj, {"type", "gateway_ip", "gateway_port", "local_ip"})
        if obj["type"] != "tunneling":
            raise ValidationError("connection.type must be 'tunneling'")
        local_ip = obj["local_ip"]
        if local_ip is not None:
            local_ip = validate_ip(local_ip, "local_ip")
        return cls(
            validate_ip(obj["gateway_ip"]),
            _integer(obj["gateway_port"], "gateway_port", minimum=1, maximum=65535),
            local_ip,
        )

    def to_obj(self) -> dict[str, Any]:
        return {
            "type": "tunneling",
            "gateway_ip": self.gateway_ip,
            "gateway_port": self.gateway_port,
            "local_ip": self.local_ip,
        }


@dataclass(frozen=True, slots=True)
class BridgeHello:
    bridge: str
    bridge_version: str
    xknx_version: str

    type: ClassVar[str] = "bridge_hello"

    def __post_init__(self) -> None:
        if self.bridge != "xknx":
            raise ValidationError("bridge must be 'xknx'")
        _string(self.bridge_version, "bridge_version")
        _string(self.xknx_version, "xknx_version")

    @classmethod
    def from_obj(cls, value: Any) -> BridgeHello:
        obj = _require_object(value, "bridge_hello")
        _require_keys(obj, {"v", "type", "bridge", "bridge_version", "xknx_version"})
        return cls(
            _string(obj["bridge"], "bridge"),
            _string(obj["bridge_version"], "bridge_version"),
            _string(obj["xknx_version"], "xknx_version"),
        )

    def to_obj(self) -> dict[str, Any]:
        return {
            "v": PROTOCOL_VERSION,
            "type": self.type,
            "bridge": self.bridge,
            "bridge_version": self.bridge_version,
            "xknx_version": self.xknx_version,
        }


@dataclass(frozen=True, slots=True)
class Configure:
    connection: TunnelingConnection
    group_addresses: tuple[GroupAddressDpt, ...]

    type: ClassVar[str] = "configure"

    def __post_init__(self) -> None:
        if len(self.group_addresses) != 2:
            raise ValidationError("group_addresses must contain input then output")
        if self.group_addresses[0].dpt != DPT_1_001:
            raise ValidationError("group_addresses[0].dpt must be 1.001")
        if self.group_addresses[1].dpt != DPT_1_001:
            raise ValidationError("group_addresses[1].dpt must be 1.001")
        if self.input_address == self.output_address:
            raise ValidationError("input and output group addresses must differ")

    @property
    def input_address(self) -> str:
        return self.group_addresses[0].address

    @property
    def output_address(self) -> str:
        return self.group_addresses[1].address

    @classmethod
    def from_obj(cls, value: Any) -> Configure:
        obj = _require_object(value, "configure")
        _require_keys(obj, {"v", "type", "connection", "group_addresses"})
        addresses = obj["group_addresses"]
        if type(addresses) is not list:
            raise ProtocolError("group_addresses must be an array")
        return cls(
            TunnelingConnection.from_obj(obj["connection"]),
            tuple(GroupAddressDpt.from_obj(entry) for entry in addresses),
        )

    def to_obj(self) -> dict[str, Any]:
        return {
            "v": PROTOCOL_VERSION,
            "type": self.type,
            "connection": self.connection.to_obj(),
            "group_addresses": [entry.to_obj() for entry in self.group_addresses],
        }


@dataclass(frozen=True, slots=True)
class Ready:
    transport: str
    gateway: str

    type: ClassVar[str] = "ready"

    def __post_init__(self) -> None:
        if self.transport != "knxip_tunneling":
            raise ValidationError("transport must be 'knxip_tunneling'")
        validate_ip(self.gateway, "gateway")

    @classmethod
    def from_obj(cls, value: Any) -> Ready:
        obj = _require_object(value, "ready")
        _require_keys(obj, {"v", "type", "transport", "gateway"})
        return cls(_string(obj["transport"], "transport"), validate_ip(obj["gateway"], "gateway"))

    def to_obj(self) -> dict[str, Any]:
        return {
            "v": PROTOCOL_VERSION,
            "type": self.type,
            "transport": self.transport,
            "gateway": self.gateway,
        }


_EVENT_SERVICES = {"group_value_write", "group_value_response", "group_value_read"}


@dataclass(frozen=True, slots=True)
class KnxEvent:
    source: str | None
    destination: str
    service: str
    dpt: Dpt
    value: BoolValue | None

    type: ClassVar[str] = "knx_event"

    def __post_init__(self) -> None:
        if self.source is not None:
            source = validate_individual_address(self.source)
            if source != self.source:
                raise ValidationError("source must be canonical")
        destination = validate_group_address(self.destination, "destination")
        if destination != self.destination:
            raise ValidationError("destination must be canonical")
        if self.service not in _EVENT_SERVICES:
            raise ValidationError("unsupported KNX service")
        if self.dpt != DPT_1_001:
            raise ValidationError("only DPT 1.001 is supported")
        if self.service == "group_value_read" and self.value is not None:
            raise ValidationError("group_value_read must not carry a value")
        if self.service != "group_value_read" and self.value is None:
            raise ValidationError(f"{self.service} must carry a value")

    @classmethod
    def from_obj(cls, value: Any) -> KnxEvent:
        obj = _require_object(value, "knx_event")
        _require_keys(obj, {"v", "type", "source", "destination", "service", "dpt", "value"})
        source = obj["source"]
        if source is not None:
            source = validate_individual_address(source)
        event_value = None if obj["value"] is None else BoolValue.from_obj(obj["value"])
        return cls(
            source,
            validate_group_address(obj["destination"], "destination"),
            _string(obj["service"], "service"),
            Dpt.from_obj(obj["dpt"]),
            event_value,
        )

    def to_obj(self) -> dict[str, Any]:
        return {
            "v": PROTOCOL_VERSION,
            "type": self.type,
            "source": self.source,
            "destination": self.destination,
            "service": self.service,
            "dpt": self.dpt.to_obj(),
            "value": None if self.value is None else self.value.to_obj(),
        }


@dataclass(frozen=True, slots=True)
class KnxWrite:
    request_id: int
    destination: str
    dpt: Dpt
    value: BoolValue

    type: ClassVar[str] = "knx_write"

    def __post_init__(self) -> None:
        _integer(self.request_id, "request_id", minimum=0)
        destination = validate_group_address(self.destination, "destination")
        if destination != self.destination:
            raise ValidationError("destination must be canonical")
        if self.dpt != DPT_1_001:
            raise ValidationError("only DPT 1.001 is supported")

    @classmethod
    def from_obj(cls, value: Any) -> KnxWrite:
        obj = _require_object(value, "knx_write")
        _require_keys(obj, {"v", "type", "request_id", "destination", "dpt", "value"})
        return cls(
            _integer(obj["request_id"], "request_id", minimum=0),
            validate_group_address(obj["destination"], "destination"),
            Dpt.from_obj(obj["dpt"]),
            BoolValue.from_obj(obj["value"]),
        )

    def to_obj(self) -> dict[str, Any]:
        return {
            "v": PROTOCOL_VERSION,
            "type": self.type,
            "request_id": self.request_id,
            "destination": self.destination,
            "dpt": self.dpt.to_obj(),
            "value": self.value.to_obj(),
        }


@dataclass(frozen=True, slots=True)
class CommandResult:
    request_id: int
    ok: bool
    error: str | None = None

    type: ClassVar[str] = "command_result"

    def __post_init__(self) -> None:
        _integer(self.request_id, "request_id", minimum=0)
        _boolean(self.ok, "ok")
        if self.error is not None:
            _string(self.error, "error")
        if not self.ok and self.error is None:
            raise ValidationError("failed command_result requires error")
        if self.ok and self.error is not None:
            raise ValidationError("successful command_result must not carry error")

    @classmethod
    def from_obj(cls, value: Any) -> CommandResult:
        obj = _require_object(value, "command_result")
        _require_keys(obj, {"v", "type", "request_id", "ok"}, {"error"})
        error = obj.get("error")
        if error is not None:
            error = _string(error, "error")
        return cls(
            _integer(obj["request_id"], "request_id", minimum=0),
            _boolean(obj["ok"], "ok"),
            error,
        )

    def to_obj(self) -> dict[str, Any]:
        result: dict[str, Any] = {
            "v": PROTOCOL_VERSION,
            "type": self.type,
            "request_id": self.request_id,
            "ok": self.ok,
        }
        if self.error is not None:
            result["error"] = self.error
        return result


@dataclass(frozen=True, slots=True)
class Fatal:
    code: str
    message: str

    type: ClassVar[str] = "fatal"

    def __post_init__(self) -> None:
        _string(self.code, "code")
        _string(self.message, "message")

    @classmethod
    def from_obj(cls, value: Any) -> Fatal:
        obj = _require_object(value, "fatal")
        _require_keys(obj, {"v", "type", "code", "message"})
        return cls(_string(obj["code"], "code"), _string(obj["message"], "message"))

    def to_obj(self) -> dict[str, Any]:
        return {"v": PROTOCOL_VERSION, "type": self.type, "code": self.code, "message": self.message}


@dataclass(frozen=True, slots=True)
class Shutdown:
    type: ClassVar[str] = "shutdown"

    @classmethod
    def from_obj(cls, value: Any) -> Shutdown:
        obj = _require_object(value, "shutdown")
        _require_keys(obj, {"v", "type"})
        return cls()

    def to_obj(self) -> dict[str, Any]:
        return {"v": PROTOCOL_VERSION, "type": self.type}


Message: TypeAlias = BridgeHello | Configure | Ready | KnxEvent | KnxWrite | CommandResult | Fatal | Shutdown

_MESSAGE_TYPES: dict[str, type[Message]] = {
    "bridge_hello": BridgeHello,
    "configure": Configure,
    "ready": Ready,
    "knx_event": KnxEvent,
    "knx_write": KnxWrite,
    "command_result": CommandResult,
    "fatal": Fatal,
    "shutdown": Shutdown,
}


def _reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ProtocolError(f"duplicate JSON field: {key}")
        result[key] = value
    return result


def _reject_json_constant(value: str) -> None:
    raise ProtocolError(f"invalid JSON constant: {value}")


def parse_line(line: str | bytes) -> Message:
    """Parse exactly one strict v1 protocol line."""
    if isinstance(line, bytes):
        try:
            line = line.decode("utf-8")
        except UnicodeDecodeError as exc:
            raise ProtocolError("protocol line is not UTF-8") from exc
    if type(line) is not str or not line.strip():
        raise ProtocolError("protocol line is empty")
    try:
        value = json.loads(
            line,
            object_pairs_hook=_reject_duplicate_keys,
            parse_constant=_reject_json_constant,
        )
    except (json.JSONDecodeError, TypeError) as exc:
        raise ProtocolError(f"invalid protocol JSON: {exc}") from exc
    obj = _require_object(value, "message")
    if "v" not in obj:
        raise ProtocolError("missing protocol version field v (expected 1)")
    version = _integer(obj["v"], "v")
    if version != PROTOCOL_VERSION:
        raise ProtocolError(f"bridge protocol unsupported expected={PROTOCOL_VERSION} received={version}")
    message_type = _string(obj.get("type"), "type")
    message_class = _MESSAGE_TYPES.get(message_type)
    if message_class is None:
        raise ProtocolError(f"unsupported message type: {message_type}")
    return message_class.from_obj(obj)


def encode_message(message: Message) -> str:
    """Serialize a typed message to one compact NDJSON line (without newline)."""
    return json.dumps(message.to_obj(), separators=(",", ":"), ensure_ascii=False, allow_nan=False)
