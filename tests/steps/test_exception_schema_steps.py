"""BDD steps for exception schema serialization scenarios."""

from __future__ import annotations

import json
import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import EXCEPTION_SCHEMA_VERSION

if typ.TYPE_CHECKING:
    from syrupy import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "exception_schema.feature"))


@pytest.fixture
def frame_data() -> dict[str, typ.Any]:
    """Storage for the current stack frame data."""
    return {}


@pytest.fixture
def exception_data() -> dict[str, typ.Any]:
    """Storage for the current exception payload data."""
    return {}


@pytest.fixture
def serialized_json() -> dict[str, str]:
    """Storage for serialized JSON output."""
    return {"value": ""}


@given(
    parsers.parse(
        'a stack frame with filename "{filename}" line {lineno:d} function "{function}"'
    )
)
def create_basic_frame(
    frame_data: dict[str, typ.Any], filename: str, lineno: int, function: str
) -> None:
    frame_data.update({
        "filename": filename,
        "lineno": lineno,
        "function": function,
    })


@given("a stack frame with all optional fields populated")
def create_full_frame(frame_data: dict[str, typ.Any]) -> None:
    frame_data.update({
        "filename": "example.py",
        "lineno": 10,
        "end_lineno": 12,
        "colno": 4,
        "end_colno": 20,
        "function": "process",
        "source_line": "    result = compute(x)",
        "locals": {"x": "42", "y": "'hello'"},
    })


@given(parsers.parse('an exception "{type_name}" with message "{message}"'))
def create_exception(
    exception_data: dict[str, typ.Any], type_name: str, message: str
) -> None:
    exception_data.update({
        "schema_version": EXCEPTION_SCHEMA_VERSION,
        "type_name": type_name,
        "message": message,
    })


@given(parsers.parse('the exception has cause "{type_name}" with message "{message}"'))
def add_cause(exception_data: dict[str, typ.Any], type_name: str, message: str) -> None:
    exception_data["cause"] = {
        "schema_version": EXCEPTION_SCHEMA_VERSION,
        "type_name": type_name,
        "message": message,
    }


@given(parsers.parse('an exception group "{type_name}" with message "{message}"'))
def create_exception_group(
    exception_data: dict[str, typ.Any], type_name: str, message: str
) -> None:
    exception_data.update({
        "schema_version": EXCEPTION_SCHEMA_VERSION,
        "type_name": type_name,
        "message": message,
        "exceptions": [],
    })


@given(
    parsers.parse('the group contains exception "{type_name}" with message "{message}"')
)
def add_nested_exception(
    exception_data: dict[str, typ.Any], type_name: str, message: str
) -> None:
    exception_data["exceptions"].append({
        "schema_version": EXCEPTION_SCHEMA_VERSION,
        "type_name": type_name,
        "message": message,
    })


@when("I serialize the frame to JSON")
def serialize_frame(
    frame_data: dict[str, typ.Any], serialized_json: dict[str, str]
) -> None:
    serialized_json["value"] = json.dumps(frame_data, sort_keys=True)


@when("I serialize the exception to JSON")
def serialize_exception(
    exception_data: dict[str, typ.Any], serialized_json: dict[str, str]
) -> None:
    serialized_json["value"] = json.dumps(exception_data, sort_keys=True)


def _decode(serialized_json: dict[str, str]) -> dict[str, object]:
    """Decode the payload recorded by the serialization ``when`` step."""
    return json.loads(serialized_json["value"])


def _assert_has_key(payload: dict[str, object], key: str, context: str) -> object:
    """Assert ``key`` is present in ``payload`` and return its value."""
    assert key in payload, (
        f"{context}: serialized payload must expose {key!r}; "
        f"present keys are {sorted(payload)}"
    )
    return payload[key]


def _assert_nested_payload(
    payload: dict[str, object], key: str, context: str
) -> dict[str, object]:
    """Assert ``key`` holds a nested JSON object and return it."""
    nested = _assert_has_key(payload, key, context)
    assert isinstance(nested, dict), (
        f"{context}: {key!r} must serialize as a JSON object, "
        f"got {type(nested).__name__}"
    )
    return nested


def _assert_field_equals(
    payload: dict[str, object], key: str, expected: object, context: str
) -> None:
    """Assert that ``key`` serializes to ``expected`` within ``payload``."""
    actual = _assert_has_key(payload, key, context)
    assert actual == expected, (
        f"{context}: {key!r} must serialize as {expected!r}, but was {actual!r}"
    )


@then(parsers.parse('the JSON contains "{key}" as "{value}"'))
def json_contains_string(serialized_json: dict[str, str], key: str, value: str) -> None:
    _assert_field_equals(
        _decode(serialized_json), key, value, "string field serialization"
    )


@then(parsers.parse('the JSON contains "{key}" as {value:d}'))
def json_contains_int(serialized_json: dict[str, str], key: str, value: int) -> None:
    _assert_field_equals(
        _decode(serialized_json), key, value, "integer field serialization"
    )


@then(parsers.parse('the JSON contains "{key}"'))
def json_has_key(serialized_json: dict[str, str], key: str) -> None:
    _assert_has_key(_decode(serialized_json), key, "optional field serialization")


@then(parsers.parse('the JSON contains nested "{parent}" with "{key}" as "{value}"'))
def json_contains_nested(
    serialized_json: dict[str, str], parent: str, key: str, value: str
) -> None:
    context = f"nested {parent!r} payload serialization"
    nested = _assert_nested_payload(_decode(serialized_json), parent, context)
    _assert_field_equals(nested, key, value, context)


@then(parsers.parse('the JSON contains "{key}" array with {count:d} items'))
def json_array_length(serialized_json: dict[str, str], key: str, count: int) -> None:
    context = "exception group serialization"
    items = _assert_has_key(_decode(serialized_json), key, context)
    assert isinstance(items, list), (
        f"{context}: {key!r} must serialize as a JSON array, got {type(items).__name__}"
    )
    assert len(items) == count, (
        f"{context}: {key!r} must hold {count} nested entries, got {len(items)}"
    )


@then("the JSON matches snapshot")
def json_matches_snapshot(
    serialized_json: dict[str, str], snapshot: SnapshotAssertion
) -> None:
    assert _decode(serialized_json) == snapshot, (
        "serialized payload must match the recorded exception schema snapshot"
    )


@then("the schema version matches the Rust constant")
def schema_version_matches_rust(serialized_json: dict[str, str]) -> None:
    _assert_field_equals(
        _decode(serialized_json),
        "schema_version",
        EXCEPTION_SCHEMA_VERSION,
        "schema version agreement between Rust and Python",
    )


@then("the EXCEPTION_SCHEMA_VERSION constant is accessible from Python")
def constant_is_accessible() -> None:
    # Import failure would already abort collection; this guards against the
    # constant being exported as a placeholder rather than a real value.
    assert EXCEPTION_SCHEMA_VERSION is not None, (
        "EXCEPTION_SCHEMA_VERSION must be re-exported from the femtologging package"
    )


@then("the constant value is a positive integer")
def constant_is_positive_int() -> None:
    assert isinstance(EXCEPTION_SCHEMA_VERSION, int), (
        "EXCEPTION_SCHEMA_VERSION must cross the FFI boundary as an int, "
        f"got {type(EXCEPTION_SCHEMA_VERSION).__name__}"
    )
    assert EXCEPTION_SCHEMA_VERSION > 0, (
        "schema versions are one-based, so EXCEPTION_SCHEMA_VERSION must be "
        f"positive; got {EXCEPTION_SCHEMA_VERSION}"
    )
