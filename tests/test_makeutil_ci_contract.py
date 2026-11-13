"""Contract tests for how CI workflows provision the Makefile parser.

Makeutil parses the Makefile into structured rules and variables, so the
contract suite can assert interfaces without depending on whitespace or nearby
source text; every full-suite job must install it.
"""

from __future__ import annotations

import typing as typ

from tests import makeutil_contract
from tests.make_contract_helpers import (
    mapping,
    objects,
    sole_workflow_step,
    workflow_job,
)

# Every full-suite job must install the prebuilt Makefile parser itself.
_MAKEUTIL_WORKFLOW_JOBS: typ.Final = (
    (".github/workflows/ci.yml", "build-test"),
    (".github/workflows/heavy-tests.yml", "heavy"),
)


def test_ci_installs_makeutil_for_each_full_suite() -> None:
    """CI must provide the Makefile parser to every full-suite job."""
    for workflow_path, job_name in _MAKEUTIL_WORKFLOW_JOBS:
        job = workflow_job(workflow_path, job_name)
        environment = mapping(
            job.get("env", {}), subject=f"{workflow_path} {job_name} environment"
        )
        for key in ("MAKEUTIL_REVISION", "MAKEUTIL_TOOLCHAIN"):
            assert key not in environment, (
                f"{workflow_path} {job_name} must not carry the from-source {key}"
            )
        contract = f"{workflow_path} {job_name} Makeutil"
        install_step = sole_workflow_step(workflow_path, job_name, "Install makeutil")
        makeutil_contract.assert_installation(install_step, contract=contract)
        makeutil_contract.assert_verification(
            install_step,
            sole_workflow_step(workflow_path, job_name, "Verify makeutil"),
            contract=contract,
        )
        makeutil_contract.assert_verification_follows_install(
            objects(job.get("steps"), subject=f"{contract} steps"),
            contract=contract,
        )
