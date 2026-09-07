"""Behaviour-driven tests for handler builders.

``tests/features/handler_builders.feature`` covers file, rotating, stream,
socket, and HTTP handler builders in one feature, so ``scenarios()`` must be
called from a single module. The step definitions themselves are grouped by
handler family in the sibling ``handler_builders_*_steps`` modules and
star-imported here: ``pytest_bdd``'s ``@given``/``@when``/``@then``
decorators inject their step fixtures into the defining module's namespace,
so a star import is what makes those fixtures resolvable from the module
that owns the scenario bindings.
"""

from __future__ import annotations

from pytest_bdd import scenarios

from tests.steps.handler_builders_file_steps import *  # ruff: ignore[undefined-local-with-import-star] pytest-bdd step fixtures must land in this module's namespace
from tests.steps.handler_builders_http_steps import *  # ruff: ignore[undefined-local-with-import-star] pytest-bdd step fixtures must land in this module's namespace
from tests.steps.handler_builders_stream_socket_steps import *  # ruff: ignore[undefined-local-with-import-star] pytest-bdd step fixtures must land in this module's namespace
from tests.steps.handler_builders_support import FEATURE_FILE

scenarios(FEATURE_FILE)
