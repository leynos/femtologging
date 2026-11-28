"""Shared Hypothesis strategies for traceback normalization properties."""

from __future__ import annotations

from hypothesis import given
from hypothesis import strategies as st

from tests.steps.traceback_normalization import SYSTEM_EXIT_PYTEST_LINES

_ENTRYPOINT_PROPERTY_CASES = given(
    segment=st.text(
        alphabet=st.characters(
            exclude_characters='"\n\r\v\f\x1c\x1d\x1e\x85\u2028\u2029'
        ),
        max_size=120,
    ),
    line_no=st.integers(min_value=1, max_value=999_999),
    entrypoint_line=st.sampled_from(sorted(SYSTEM_EXIT_PYTEST_LINES)),
)
"""Decorator for entrypoint property cases."""

_LINE_PROPERTY_CASES = given(
    bodies=st.lists(
        st.text(
            alphabet=st.characters(
                exclude_characters="\r\n\v\f\x1c\x1d\x1e\x85\u2028\u2029"
            ),
            max_size=40,
        ),
        max_size=6,
    ),
    ending=st.sampled_from(["\n", "\r\n"]),
    ends_with_terminator=st.booleans(),
)
"""Decorator for line-content property cases."""

_FRAME_PATH_PROPERTY_CASES = given(
    segment=st.text(
        alphabet=st.characters(
            categories=("L", "N", "P", "S", "Zs"),
            exclude_characters='"/\\<>:|?*.\x00',
        ),
        max_size=40,
    ),
)
"""Decorator for frame-path property cases."""
