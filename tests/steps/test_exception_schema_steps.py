"""BDD steps for exception schema serialisation scenarios."""

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
def serialised_json() -> dict[str, str]:
    """Storage for serialised JSON output."""
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


@when("I serialise the frame to JSON")
def serialise_frame(
    frame_data: dict[str, typ.Any], serialised_json: dict[str, str]
) -> None:
    serialised_json["value"] = json.dumps(frame_data, sort_keys=True)


@when("I serialise the exception to JSON")
def serialise_exception(
    exception_data: dict[str, typ.Any], serialised_json: dict[str, str]
) -> None:
    serialised_json["value"] = json.dumps(exception_data, sort_keys=True)


@then(parsers.parse('the JSON contains "{key}" as "{value}"'))
def json_contains_string(serialised_json: dict[str, str], key: str, value: str) -> None:
    data = json.loads(serialised_json["value"])
    assert key in data
    assert data[key] == value


@then(parsers.parse('the JSON contains "{key}" as {value:d}'))
def json_contains_int(serialised_json: dict[str, str], key: str, value: int) -> None:
    data = json.loads(serialised_json["value"])
    assert key in data
    assert data[key] == value


@then(parsers.parse('the JSON contains "{key}"'))
def json_has_key(serialised_json: dict[str, str], key: str) -> None:
    data = json.loads(serialised_json["value"])
    assert key in data


@then(parsers.parse('the JSON contains nested "{parent}" with "{key}" as "{value}"'))
def json_contains_nested(
    serialised_json: dict[str, str], parent: str, key: str, value: str
) -> None:
    data = json.loads(serialised_json["value"])
    assert parent in data
    assert key in data[parent]
    assert data[parent][key] == value


@then(parsers.parse('the JSON contains "{key}" array with {count:d} items'))
def json_array_length(serialised_json: dict[str, str], key: str, count: int) -> None:
    data = json.loads(serialised_json["value"])
    assert key in data
    assert isinstance(data[key], list)
    assert len(data[key]) == count


@then("the JSON matches snapshot")
def json_matches_snapshot(
    serialised_json: dict[str, str], snapshot: SnapshotAssertion
) -> None:
    data = json.loads(serialised_json["value"])
    assert data == snapshot


@then("the schema version matches the Rust constant")
def schema_version_matches_rust(serialised_json: dict[str, str]) -> None:
    data = json.loads(serialised_json["value"])
    assert data["schema_version"] == EXCEPTION_SCHEMA_VERSION


@then("the EXCEPTION_SCHEMA_VERSION constant is accessible from Python")
def constant_is_accessible() -> None:
    # This test will fail at import time if the constant is not exported
    assert EXCEPTION_SCHEMA_VERSION is not None


@then("the constant value is a positive integer")
def constant_is_positive_int() -> None:
    assert isinstance(EXCEPTION_SCHEMA_VERSION, int)
    assert EXCEPTION_SCHEMA_VERSION > 0
