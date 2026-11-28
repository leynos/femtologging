"""Property-based checks for bounded exception payload capture.

The example tests pin exact marker text; these properties check the invariant
behind it: however large or numerous the caller-supplied values are, one
logging call still returns bounded text and never propagates a conversion
failure.
"""

from __future__ import annotations

from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

from femtologging import FemtoLogger

_MAX_OUTPUT = 40_000


@settings(
    max_examples=25,
    deadline=None,
    suppress_health_check=[HealthCheck.function_scoped_fixture],
)
@given(
    message=st.text(max_size=8_000),
    extra_args=st.lists(st.integers(), max_size=40),
)
def test_huge_messages_and_arguments_stay_bounded(
    message: str, extra_args: list[int]
) -> None:
    """A message and argument tuple of any generated size must stay bounded."""
    logger = FemtoLogger("properties.bounded-args")

    output = logger.log("INFO", "property", exc_info=ValueError(message, *extra_args))

    assert output is not None, "an accepted record must produce output"
    assert len(output) < _MAX_OUTPUT, "the formatted text must stay bounded"


@settings(max_examples=20, deadline=None)
@given(note_count=st.integers(min_value=0, max_value=200))
def test_many_notes_stay_bounded_and_marked(note_count: int) -> None:
    """Any note count must produce bounded text with the omission reported."""
    exc = ValueError("noted")
    for index in range(note_count):
        exc.add_note(f"note {index}")
    logger = FemtoLogger("properties.bounded-notes")

    output = logger.log("INFO", "property", exc_info=exc)

    assert output is not None, "an accepted record must produce output"
    assert len(output) < _MAX_OUTPUT, "the formatted text must stay bounded"
    if note_count > 32:
        assert "notes omitted" in output, "the note ceiling must be reported"
