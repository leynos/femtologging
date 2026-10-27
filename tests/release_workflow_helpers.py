"""Test helpers for the checked-in release workflow scripts."""

from __future__ import annotations

import os
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] the tests execute checked-in workflow scripts
import sys
import typing as typ

import yaml

from tests.make_contract_helpers import (
    mapping,
    objects,
    repo_root,
    sole_workflow_step,
    workflow_job,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

ROOT = repo_root()


def step_script(workflow: str, job_name: str, step_name: str) -> str:
    """Return the shell script from one named workflow step."""
    step = sole_workflow_step(workflow, job_name, step_name)
    script = step.get("run")
    if not isinstance(script, str):
        message = f"{workflow} {job_name}/{step_name} has no run script"
        raise TypeError(message)
    return script


def _is_literal_environment_entry(key: object, value: object) -> bool:
    """Return whether one environment entry has a literal string value."""
    if not isinstance(key, str):
        return False
    if not isinstance(value, str):
        return False
    return "${{" not in value


def step_environment(workflow: str, job_name: str, step_name: str) -> dict[str, str]:
    """Merge literal job and step environment values for one shell step."""
    job = workflow_job(workflow, job_name)
    step = sole_workflow_step(workflow, job_name, step_name)
    result: dict[str, str] = {}
    for source in (job.get("env", {}), step.get("env", {})):
        if isinstance(source, dict):
            entries = {
                key: value
                for key, value in source.items()
                if _is_literal_environment_entry(key, value)
            }
            result.update(typ.cast("dict[str, str]", entries))
    return result


def workflow_environment_mappings(workflow: str) -> list[dict[str, object]]:
    """Collect the workflow, job, and step environment mappings.

    Parameters
    ----------
    workflow : str
        Repository-relative path to the parsed workflow document.

    Returns
    -------
    list[dict[str, object]]
        All environment mappings, including empty mappings at unset levels.

    Examples
    --------
    >>> isinstance(workflow_environment_mappings(".github/workflows/ci.yml"), list)
    True
    """
    document = mapping(
        yaml.safe_load((ROOT / workflow).read_text(encoding="utf-8")),
        subject=workflow,
    )
    jobs = mapping(document.get("jobs"), subject=f"{workflow} jobs")
    job_documents = [
        mapping(job, subject=f"{workflow} {name} job") for name, job in jobs.items()
    ]
    step_documents: list[dict[str, object]] = []
    for job_document in job_documents:
        step_documents.extend(
            objects(job_document.get("steps", []), subject="workflow steps")
        )
    levels = [document, *job_documents, *step_documents]
    return [
        mapping(level.get("env", {}), subject=f"{workflow} environment")
        for level in levels
    ]


def run_bash(
    script: str,
    cwd: Path,
    environment: cabc.Mapping[str, str] | None = None,
    *,
    timeout: float = 30,
) -> subprocess.CompletedProcess[str]:
    """Run an extracted workflow script in a bounded Bash subprocess."""
    env = dict(os.environ)
    for name in ("BASH_ENV", "GH_TOKEN", "GITHUB_TOKEN", "GITHUB_OUTPUT"):
        env.pop(name, None)
    env.pop("BASH_FUNC_gh%%", None)
    env.update(environment or {})
    bash_path = shutil.which("bash")
    if bash_path is None:
        message = "Bash is required to exercise the release workflow scripts"
        raise FileNotFoundError(message)
    return subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] Bash executes the checked-in workflow script
        [bash_path, "--noprofile", "--norc", "-c", script],
        capture_output=True,
        check=False,
        cwd=cwd,
        env=env,
        text=True,
        timeout=timeout,
    )


def copy_release_scripts(directory: Path) -> Path:
    """Copy the release helpers into a scratch workspace for workflow steps."""
    scripts = directory / "scripts"
    scripts.mkdir(parents=True, exist_ok=True)
    for name in (
        "release_asset_errors.py",
        "release_asset_io.py",
        "release_asset_plan.py",
        "release_assets.py",
        "release_assets_cli.py",
        "release_version.py",
        "fetch_pypi_index.sh",
    ):
        source = ROOT / "scripts" / name
        target = scripts / name
        shutil.copy2(source, target)
    return scripts


def workflow_command_environment(
    step: tuple[str, str, str],
    directory: Path,
    overrides: cabc.Mapping[str, str] | None = None,
) -> dict[str, str]:
    """Prepare a named step's literal environment with isolated output paths."""
    copy_release_scripts(directory)
    return {
        **step_environment(*step),
        "GITHUB_OUTPUT": str(directory / "github-output"),
        **(overrides or {}),
    }


def output_values(path: Path) -> dict[str, str]:
    """Read ``name=value`` lines written to a GitHub Actions output file."""
    if not path.exists():
        return {}
    return dict(
        line.split("=", maxsplit=1)
        for line in path.read_text(encoding="utf-8").splitlines()
    )


def write_argument_recorder(directory: Path, name: str, body: str) -> Path:
    """Create an executable Python tool with a checked-in caller-supplied body."""
    directory.mkdir(parents=True, exist_ok=True)
    executable = directory / name
    executable.write_text(f"#!{sys.executable}\n{body}", encoding="utf-8")
    executable.chmod(0o755)
    return executable
