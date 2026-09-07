"""Contract tests keeping the Makefile and CI tool pins in sync.

Ruff and ty are pinned in two places: the Makefile defaults drive local runs,
and the CI workflow's job environment repeats the pin so the workflow file
records the version it tests with. These tests assert the two stay identical
without asserting any specific version, so a deliberate bump only has to touch
the two pins and no test constant.
"""

from __future__ import annotations

import pytest

from tests.make_contract_helpers import variable_tokens, workflow_job

_CI_WORKFLOW = ".github/workflows/ci.yml"
_CI_JOB = "build-test"


def _ci_environment_pin(name: str) -> str:
    """Return the named version pin from the CI job environment."""
    job = workflow_job(_CI_WORKFLOW, _CI_JOB)
    environment = job.get("env")
    assert isinstance(environment, dict), (
        f"{_CI_WORKFLOW} job {_CI_JOB!r} must define a job environment"
    )
    value = environment.get(name)
    assert isinstance(value, str), (
        f"{_CI_WORKFLOW} job {_CI_JOB!r} must pin {name} as a string"
    )
    assert value.strip(), (
        f"{_CI_WORKFLOW} job {_CI_JOB!r} must pin {name} in its environment"
    )
    return value


@pytest.mark.parametrize(
    "variable_name",
    ["RUFF_VERSION", "TY_VERSION"],
)
def test_makefile_and_ci_tool_pins_stay_in_sync(variable_name: str) -> None:
    """The Makefile default and the CI environment must pin the same release."""
    makefile_tokens = variable_tokens(variable_name)
    assert len(makefile_tokens) == 1, (
        f"Makefile {variable_name} must hold exactly one version token"
    )
    makefile_pin = makefile_tokens[0]
    assert makefile_pin, f"Makefile {variable_name} must not be empty"

    ci_pin = _ci_environment_pin(variable_name)
    assert ci_pin == makefile_pin, (
        f"{variable_name} must match between the Makefile default "
        f"({makefile_pin!r}) and the {_CI_JOB} job environment ({ci_pin!r})"
    )
