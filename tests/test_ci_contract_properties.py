"""Property tests for the CI contract rules added with the Loom gate.

Two kinds of invariant are generated rather than tabulated. The matrix
tolerance rule is checked against an independent model of GitHub's expansion,
so a reader cannot agree with its own helpers; the formatter-gate rule is
checked against generated command suffixes, so the reason a suffix is refused
is the rule under test and not a remembered example.
"""

from __future__ import annotations

import typing as typ

from hypothesis import given, settings
from hypothesis import strategies as st

from tests.ci_gate_scope_contract import job_tolerance_is_acceptable
from tests.heavy_formatter_gate_contract import formatter_command_problems
from tests.heavy_workflow_document import FORMATTERS_STEP

if typ.TYPE_CHECKING:
    from tests.heavy_workflow_document import Job

#: Axis names that differ only in case, and their values.
_AXIS_NAMES: typ.Final[tuple[str, ...]] = (
    "experimental",
    "Experimental",
    "EXPERIMENTAL",
)

#: Suffixes that change what a described gate command does without changing
#: the words it starts with.
_MASKS: typ.Final[tuple[str, ...]] = (
    " || true",
    " || :",
    " | tee /dev/null",
    " | true",
    " &",
    " & wait",
    " --collect-only",
    " --cap-lints allow",
)


def _job(*, exclude: list[dict[str, object]] | None) -> Job:
    """Return a job whose tolerance reads a mixed-leg matrix property."""
    matrix: dict[str, object] = {"experimental": [False, True]}
    if exclude is not None:
        matrix["exclude"] = exclude
    return {
        "continue-on-error": "${{ matrix.experimental }}",
        "strategy": {"matrix": matrix},
    }


@settings(max_examples=50, deadline=None)
@given(name=st.sampled_from(_AXIS_NAMES), tolerate=st.booleans())
def test_a_single_mixed_case_exclusion_always_removes_the_required_leg(
    name: str, *, tolerate: bool
) -> None:
    """An exclusion that folds to the axis removes the leg, in either case."""
    job = _job(exclude=[{name: tolerate}])
    # Excluding `False` leaves only the tolerated leg, so the gate has no
    # required leg left; excluding `True` leaves the required one, so it does.
    assert job_tolerance_is_acceptable(job) is tolerate, (name, tolerate)


@settings(max_examples=50, deadline=None)
@given(
    axis=st.sampled_from(_AXIS_NAMES),
    selector=st.sampled_from(_AXIS_NAMES),
)
def test_any_spelling_of_the_axis_and_selector_keeps_its_required_leg(
    axis: str, selector: str
) -> None:
    """A matrix without exclusions keeps a required leg in any spelling pair.

    The axis name and the selector name are generated independently, so the
    property covers GitHub's case-insensitive match rather than one fixed
    spelling with an unused generated value.
    """
    job: Job = {
        "continue-on-error": f"${{{{ matrix.{selector} }}}}",
        "strategy": {"matrix": {axis: [False, True]}},
    }
    assert job_tolerance_is_acceptable(job), (axis, selector)


def _formatter_job(run: str) -> Job:
    """Return a heavy job whose formatter step runs `run`."""
    return {
        "steps": [
            {"name": "Compile Loom heavy tests", "run": "cargo test --test heavy"},
            {"name": FORMATTERS_STEP, "run": run},
            {"name": "Run Loom models", "run": "checker"},
        ]
    }


_BASE_SCRIPT: typ.Final[str] = (
    "cargo fmt --manifest-path rust_extension/Cargo.toml -- --check\n"
    "make lint-rust-clippy\n"
    'RUSTFLAGS="--cfg loom -C link-arg=-fuse-ld=mold" cargo clippy '
    "--manifest-path rust_extension/Cargo.toml --no-default-features "
    "--all-targets -- -D warnings\n"
    "uv run --locked pytest -q\n"
)


def test_the_generated_control_script_is_accepted() -> None:
    """The unmodified generated script is the control the mutations need."""
    assert not formatter_command_problems({}, _formatter_job(_BASE_SCRIPT)), (
        "the control script must be accepted, or the property proves nothing"
    )


@settings(max_examples=50, deadline=None)
@given(
    suffix=st.sampled_from(_MASKS),
    line=st.sampled_from((
        "make lint-rust-clippy",
        "uv run --locked pytest -q",
        "--all-targets -- -D warnings",
    )),
)
def test_a_masking_suffix_on_a_gate_command_is_refused(suffix: str, line: str) -> None:
    """Appending any mask to a required line is reported, never accepted."""
    mutated = _BASE_SCRIPT.replace(line, f"{line}{suffix}")
    assert mutated != _BASE_SCRIPT, suffix
    problems = formatter_command_problems({}, _formatter_job(mutated))
    assert problems, f"{line}{suffix!r} must be reported"


@settings(max_examples=50, deadline=None)
@given(
    extra=st.text(alphabet=st.characters(whitelist_categories=("Ll", "Nd")), max_size=8)
)
def test_an_appended_argument_on_pytest_is_refused(extra: str) -> None:
    """No appended argument leaves the pytest lane matching its required line."""
    mutated = _BASE_SCRIPT.replace(
        "uv run --locked pytest -q", f"uv run --locked pytest -q --{extra}"
    )
    assert formatter_command_problems({}, _formatter_job(mutated)), extra
