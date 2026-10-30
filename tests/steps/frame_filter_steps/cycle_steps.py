"""BDD steps for self-referential exception payloads.

These steps create a cyclic payload, capture its filtering error, and assert
that the public binding rejects the graph.
"""

from __future__ import annotations

import typing as typ

from pytest_bdd import given, then, when

from femtologging import filter_frames
from tests.frame_filter.conftest import make_exception_payload

if typ.TYPE_CHECKING:
    from tests.steps.frame_filter_steps.utils import FilterFixture


@given("an exception payload with a self-referential cause")
def create_self_referential_exception(filter_fixture: FilterFixture) -> None:
    """Create an exception payload whose cause points back to itself.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared state storage for the scenario.
    """
    payload = make_exception_payload([], type_name="CycleError")
    typ.cast("dict[str, object]", payload)["cause"] = payload
    filter_fixture["payload"] = payload


@when("I filter the exception payload")
def filter_exception_payload_with_cycle_check(filter_fixture: FilterFixture) -> None:
    """Capture the cycle rejection for the assertion step.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared state storage containing the payload to filter.
    """
    try:
        filter_frames(filter_fixture["payload"])
    except ValueError as error:
        filter_fixture["error"] = error
    else:
        filter_fixture["error"] = None


@then("filtering rejects the cycle with a ValueError")
def check_cycle_value_error(filter_fixture: FilterFixture) -> None:
    """Assert that a self-referential cause is rejected with a ValueError.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared state storage containing the captured filtering error.
    """
    error = filter_fixture.get("error")
    assert isinstance(error, ValueError), f"expected ValueError, got {error!r}"
    assert "exception payload graph contains a cycle at $.cause" in str(error), (
        f"expected cycle path in ValueError, got {error!r}"
    )
