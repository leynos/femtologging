"""BDD steps for the HTTP handler builder, including authentication.

Star-imported by ``tests/steps/test_handler_builders_steps.py``; see
``tests/steps/handler_builders_support.py`` for the rationale.
"""

from __future__ import annotations

import re
import typing as typ

import pytest
from pytest_bdd import given, parsers, then, when

from femtologging import HandlerConfigError, HTTPHandlerBuilder
from tests.steps.handler_builders_support import build_flush_close

if typ.TYPE_CHECKING:
    from syrupy import SnapshotAssertion

    from femtologging._femtologging_rs import (
        HTTPBasicAuthConfig,
        HTTPTokenAuthConfig,
    )

ENDPOINT_URL = "http://localhost:8080/log"
BEARER_MAPPING = {
    "auth_type": "bearer",
    "format": "url_encoded",
    "url": ENDPOINT_URL,
}


@given(
    parsers.parse('an HTTPHandlerBuilder for URL "{url}"'),
    target_fixture="http_builder",
)
def given_http_builder(url: str) -> HTTPHandlerBuilder:
    """Create an HTTP builder configured for the scenario's endpoint URL."""
    return HTTPHandlerBuilder().with_endpoint(url)


@given("an empty HTTPHandlerBuilder", target_fixture="http_builder")
def given_empty_http_builder() -> HTTPHandlerBuilder:
    """Create an HTTP builder without endpoint or authentication settings."""
    return HTTPHandlerBuilder()


@when("I set HTTP method POST", target_fixture="http_builder")
def when_set_http_method_post(http_builder: HTTPHandlerBuilder) -> HTTPHandlerBuilder:
    """Set the current HTTP builder's method to ``POST``."""
    current_url = typ.cast("str", http_builder.as_dict()["url"])
    return http_builder.with_endpoint(current_url, "POST")


@when(
    parsers.parse("I set HTTP connect timeout {timeout:d}"),
    target_fixture="http_builder",
)
def when_set_http_connect_timeout(
    http_builder: HTTPHandlerBuilder, timeout: int
) -> HTTPHandlerBuilder:
    """Set the HTTP connection timeout in milliseconds."""
    return http_builder.with_connect_timeout_ms(timeout)


@when(
    parsers.parse("I set HTTP write timeout {timeout:d}"),
    target_fixture="http_builder",
)
def when_set_http_write_timeout(
    http_builder: HTTPHandlerBuilder, timeout: int
) -> HTTPHandlerBuilder:
    """Set the HTTP write timeout in milliseconds."""
    return http_builder.with_write_timeout_ms(timeout)


@when("I enable JSON format", target_fixture="http_builder")
def when_enable_json_format(http_builder: HTTPHandlerBuilder) -> HTTPHandlerBuilder:
    """Configure the current HTTP builder to emit JSON records."""
    return http_builder.with_json_format()


@when(
    parsers.parse('I set basic auth user "{user}" password "{password}"'),
    target_fixture="http_builder",
)
def when_set_basic_auth(
    http_builder: HTTPHandlerBuilder, user: str, password: str
) -> HTTPHandlerBuilder:
    """Set basic authentication credentials on the HTTP builder."""
    return http_builder.with_auth({"username": user, "password": password})


@when(parsers.parse('I set bearer token "{token}"'), target_fixture="http_builder")
def when_set_bearer_token(
    http_builder: HTTPHandlerBuilder, token: str
) -> HTTPHandlerBuilder:
    """Set bearer-token authentication on the HTTP builder."""
    return http_builder.with_auth({"token": token})


@when(
    parsers.parse(
        'I set auth config token "{token}" with extra key "{key}" value "{value}"'
    ),
    target_fixture="http_builder",
)
def when_set_auth_with_extra_key(
    http_builder: HTTPHandlerBuilder, token: str, key: str, value: str
) -> HTTPHandlerBuilder:
    """Set bearer authentication while including an unsupported extra key."""
    return http_builder.with_auth({"token": token, key: value})


def _reject_auth_config(
    http_builder: HTTPHandlerBuilder,
    config: dict[str, str],
    message: str,
) -> ValueError:
    """Assert ``with_auth`` rejects *config* and return the raised error.

    Returns
    -------
    ValueError
        The validation error ``with_auth`` raised.
    """
    invalid_config = typ.cast("HTTPBasicAuthConfig | HTTPTokenAuthConfig", config)
    with pytest.raises(ValueError, match=re.escape(message)) as exc_info:
        http_builder.with_auth(invalid_config)
    return exc_info.value


@when(
    parsers.parse(
        'I try auth config token "{token}" username "{user}" password "{password}"'
    ),
    target_fixture="auth_error",
)
def when_try_mixed_auth_config(
    http_builder: HTTPHandlerBuilder, token: str, user: str, password: str
) -> ValueError:
    """Capture the validation error for mixed token and basic credentials."""
    # Intentionally mix token and basic-auth fields to exercise validation.
    return _reject_auth_config(
        http_builder,
        {"token": token, "username": user, "password": password},
        "with_auth config must not mix 'token' with 'username'/'password'",
    )


@when(
    parsers.parse('I try auth config username "{user}" without password'),
    target_fixture="auth_error",
)
def when_try_incomplete_basic_auth(
    http_builder: HTTPHandlerBuilder, user: str
) -> ValueError:
    """Capture the validation error for incomplete basic authentication."""
    # Intentionally omit password/token so with_auth rejects the payload.
    return _reject_auth_config(
        http_builder,
        {"username": user},
        "with_auth config must specify either 'token' or both "
        "'username' and 'password'",
    )


@when(parsers.parse('I set record fields to "{fields}"'), target_fixture="http_builder")
def when_set_record_fields(
    http_builder: HTTPHandlerBuilder, fields: str
) -> HTTPHandlerBuilder:
    """Set the comma-separated record fields on the HTTP builder."""
    field_list = [f.strip() for f in fields.split(",")]
    return http_builder.with_record_fields(field_list)


@then("the HTTP handler builder matches snapshot")
def then_http_builder_snapshot(
    http_builder: HTTPHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    """Compare the HTTP builder mapping with its syrupy snapshot."""
    assert http_builder.as_dict() == snapshot, "HTTP builder dict must match snapshot"
    build_flush_close(http_builder)


@then("the JSON HTTP handler builder matches snapshot")
def then_json_http_builder_snapshot(
    http_builder: HTTPHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    """Verify JSON format and compare the HTTP builder with its snapshot."""
    data = http_builder.as_dict()
    assert data.get("format") == "json", "must have JSON format"
    assert data == snapshot, "JSON HTTP builder dict must match snapshot"
    build_flush_close(http_builder)


@then("the HTTP handler builder with auth matches snapshot")
def then_http_builder_auth_snapshot(
    http_builder: HTTPHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    """Verify basic-auth mapping and compare the HTTP builder with its snapshot."""
    data = http_builder.as_dict()
    assert data == {
        "auth_type": "basic",
        "auth_user": "admin",
        "format": "url_encoded",
        "url": ENDPOINT_URL,
    }, "must expose the exact basic-auth mapping"
    assert "token" not in data, "basic auth must not retain bearer fields"
    assert data == snapshot, "HTTP builder with auth must match snapshot"
    build_flush_close(http_builder)


@then("the HTTP handler builder with bearer matches snapshot")
def then_http_builder_bearer_snapshot(
    http_builder: HTTPHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    """Verify bearer-auth mapping and compare the HTTP builder with its snapshot."""
    data = http_builder.as_dict()
    assert data == BEARER_MAPPING, "must expose the exact bearer-auth mapping"
    assert "auth_user" not in data, "bearer auth must not retain basic fields"
    assert "token" not in data, "raw token must not leak through builder output"
    assert "scope" not in data, "unsupported auth keys must be ignored"
    assert data == snapshot, "HTTP builder with bearer must match snapshot"
    build_flush_close(http_builder)


@then("the HTTP handler builder ignores unsupported auth keys")
def then_http_builder_ignores_unsupported_auth_keys(
    http_builder: HTTPHandlerBuilder,
) -> None:
    """Verify unsupported authentication keys do not change the resolved mapping."""
    data = http_builder.as_dict()
    assert data == BEARER_MAPPING, (
        "unsupported auth keys must not change the resolved bearer mapping"
    )
    assert "scope" not in data, "unsupported auth keys must be ignored"
    build_flush_close(http_builder)


@then("the HTTP handler builder with fields matches snapshot")
def then_http_builder_fields_snapshot(
    http_builder: HTTPHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    """Verify record fields and compare the HTTP builder with its snapshot."""
    data = http_builder.as_dict()
    assert "record_fields" in data, "must have record_fields"
    assert data == snapshot, "HTTP builder with fields must match snapshot"
    build_flush_close(http_builder)


@then(parsers.parse('building the HTTP handler fails with "{message}"'))
def then_http_builder_fails(http_builder: HTTPHandlerBuilder, message: str) -> None:
    """Verify that building the HTTP handler raises the expected error."""
    with pytest.raises(HandlerConfigError, match=re.escape(message)):
        http_builder.build()


@then(parsers.parse('setting the HTTP auth config fails with "{message}"'))
def then_http_auth_config_fails(auth_error: ValueError, message: str) -> None:
    """Verify that the captured HTTP authentication error has the expected text."""
    assert str(auth_error) == message, "HTTP auth config error must match exactly"
