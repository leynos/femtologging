"""BDD steps running the README and users' guide first-run examples."""

from __future__ import annotations

import typing as typ
from pathlib import Path

from pytest_bdd import given, parsers, scenarios, then, when

from tests.documentation_examples import documented_example, run_example

if typ.TYPE_CHECKING:
    import subprocess

    from tests.documentation_examples import DocumentedExample

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "documentation_examples.feature"))


@given(
    parsers.parse('the published example "{identifier}"'),
    target_fixture="example",
)
def published_example(identifier: str) -> DocumentedExample:
    return documented_example(identifier)


@when("I run it in a fresh interpreter", target_fixture="result")
def run_published_example(
    example: DocumentedExample, tmp_path: Path
) -> subprocess.CompletedProcess[str]:
    return run_example(example, tmp_path)


@then("it exits successfully")
def exits_successfully(result: subprocess.CompletedProcess[str]) -> None:
    assert result.returncode == 0, f"example failed:\n{result.stderr}"


@then(parsers.parse('it prints the output published as "{identifier}"'))
def prints_published_output(
    result: subprocess.CompletedProcess[str], identifier: str
) -> None:
    expected = documented_example(identifier).body
    assert result.stdout + result.stderr == expected, (
        f"output differs from {identifier}"
    )
