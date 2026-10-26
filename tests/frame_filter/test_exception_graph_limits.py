"""Tests for cycle detection and budgets on exception-payload graphs."""

from __future__ import annotations

import re
import typing as typ

import pytest

from femtologging import filter_frames

from .conftest import (
    ExceptionPayload,
    FilteredPayload,
    make_exception_payload,
    make_stack_payload,
)

_DEFAULT_MAX_EXCEPTION_DEPTH = 1_000
_DEFAULT_MAX_EXCEPTION_NODES = 10_000


def _cause_chain(
    node_count: int,
    keepalive: list[ExceptionPayload | None] | None = None,
) -> ExceptionPayload:
    """Build a cause chain with exactly ``node_count`` payloads."""
    root = make_exception_payload([])
    if keepalive is not None:
        keepalive.append(root)
    current = root
    for index in range(1, node_count):
        child = make_exception_payload([], type_name=f"Error{index}")
        if keepalive is not None:
            keepalive.append(child)
        current["cause"] = child
        current = child
    return root


def _walk_cause_chain(root: FilteredPayload) -> int:
    """Count a cause chain without recursing through Python frames."""
    count = 0
    current: FilteredPayload | None = root
    while current is not None:
        count += 1
        current = current.get("cause")
    return count


def _walk_exception_graph(root: FilteredPayload) -> int:
    """Count an acyclic output graph iteratively, including shared occurrences."""
    pending = [root]
    count = 0
    while pending:
        current = pending.pop()
        count += 1
        cause = current.get("cause")
        context = current.get("context")
        exceptions = current.get("exceptions", [])
        if isinstance(cause, dict):
            pending.append(cause)
        if isinstance(context, dict):
            pending.append(context)
        if isinstance(exceptions, list):
            pending.extend(member for member in exceptions if isinstance(member, dict))
    return count


def _snapshot_exception_node(
    path: str,
    node: ExceptionPayload,
) -> tuple[tuple[object, ...], list[tuple[str, ExceptionPayload]]]:
    """Return one node's snapshot and reverse-order traversal entries."""
    cause = node.get("cause")
    context = node.get("context")
    exceptions = node.get("exceptions", [])
    frame_values = tuple(
        tuple(sorted(frame.items())) for frame in node.get("frames", [])
    )
    snapshot_entry = (
        path,
        node.get("type_name"),
        node.get("message"),
        frame_values,
        id(cause) if cause is not None else None,
        id(context) if context is not None else None,
        tuple(id(member) for member in exceptions),
    )
    pending = [
        (f"{path}.exceptions[{index}]", member)
        for index, member in reversed(list(enumerate(exceptions)))
    ]
    if context is not None:
        pending.append((f"{path}.context", context))
    if cause is not None:
        pending.append((f"{path}.cause", cause))
    return snapshot_entry, pending


def _graph_snapshot(root: ExceptionPayload) -> tuple[tuple[object, ...], ...]:
    """Record a cyclic graph's contents and edges with an iterative walk."""
    pending: list[tuple[str, ExceptionPayload]] = [("$", root)]
    seen: set[int] = set()
    snapshot: list[tuple[object, ...]] = []

    while pending:
        path, node = pending.pop()
        identity = id(node)
        if identity in seen:
            continue
        seen.add(identity)
        snapshot_entry, nested_nodes = _snapshot_exception_node(path, node)
        snapshot.append(snapshot_entry)
        pending.extend(nested_nodes)

    return tuple(snapshot)


@pytest.mark.parametrize(
    ("edge", "cycle_path"),
    [
        ("cause", "$.cause"),
        ("context", "$.context"),
        ("exceptions", "$.exceptions[0]"),
    ],
    ids=["cause", "context", "exception-group"],
)
def test_filter_frames_rejects_self_cycles_without_mutating_input(
    edge: str,
    cycle_path: str,
) -> None:
    """Self-cycles fail with their edge path and leave the input unchanged."""
    payload = make_exception_payload([])
    raw_payload = typ.cast("dict[str, object]", payload)
    raw_payload[edge] = [payload] if edge == "exceptions" else payload
    before = _graph_snapshot(payload)

    with pytest.raises(
        ValueError,
        match=re.escape(f"exception payload graph contains a cycle at {cycle_path}"),
    ):
        filter_frames(payload)

    assert _graph_snapshot(payload) == before, (
        "cycle rejection must not mutate the input graph"
    )


def test_filter_frames_rejects_two_node_cause_context_cycle() -> None:
    """Cycles crossing two dictionaries report the full active path."""
    first = make_exception_payload([])
    second = make_exception_payload([], type_name="OSError")
    first["cause"] = second
    typ.cast("dict[str, object]", second)["context"] = first
    before = _graph_snapshot(first)

    with pytest.raises(
        ValueError,
        match=re.escape("exception payload graph contains a cycle at $.cause.context"),
    ):
        filter_frames(first)

    assert _graph_snapshot(first) == before, (
        "cycle rejection must not mutate the input graph"
    )


def test_filter_frames_rejects_cycle_below_nested_group_prefix() -> None:
    """Cycle paths include cause, exception-group index and context edges."""
    root = make_exception_payload([])
    middle = make_exception_payload([], type_name="ExceptionGroup")
    leaf = make_exception_payload([], type_name="ValueError")
    root["cause"] = middle
    middle["exceptions"] = [leaf]
    typ.cast("dict[str, object]", leaf)["context"] = root

    with pytest.raises(
        ValueError,
        match=re.escape(
            "exception payload graph contains a cycle at $.cause.exceptions[0].context"
        ),
    ):
        filter_frames(root)


def test_filter_frames_reports_group_cycle_before_depth_overflow() -> None:
    """A group cycle at the depth boundary keeps its cycle-specific error."""
    payload = make_exception_payload([])
    typ.cast("dict[str, object]", payload)["exceptions"] = [payload]

    with pytest.raises(
        ValueError,
        match=re.escape("exception payload graph contains a cycle at $.exceptions[0]"),
    ):
        filter_frames(payload, max_exception_depth=0)


def test_filter_frames_copies_shared_acyclic_cause_and_context() -> None:
    """A shared dictionary is accepted and copied once for each edge."""
    shared = make_exception_payload(
        ["shared.py", "logging/__init__.py"],
        type_name="OSError",
    )
    root = make_exception_payload([])
    raw_root = typ.cast("dict[str, object]", root)
    raw_root["cause"] = shared
    raw_root["context"] = shared

    filtered = filter_frames(root, exclude_logging=True)

    assert filtered["cause"] is not filtered["context"], (
        "shared cause and context occurrences must be copied separately"
    )
    assert filtered["cause"]["frames"] == [shared["frames"][0]], (
        "the cause occurrence must filter shared frames"
    )
    assert filtered["context"]["frames"] == [shared["frames"][0]], (
        "the context occurrence must filter shared frames"
    )


def test_filter_frames_copies_shared_exception_group_member_per_occurrence() -> None:
    """Repeated group members produce independent filtered dictionaries."""
    shared = make_exception_payload(["shared.py"])
    root = make_exception_payload([])
    root["exceptions"] = [shared, shared]

    filtered = filter_frames(root)

    first, second = filtered["exceptions"]
    assert first is not second, (
        "repeated exception-group members must be copied separately"
    )
    assert first["frames"] == second["frames"], (
        "repeated exception-group members must be filtered consistently"
    )


def test_filter_frames_enforces_the_default_exception_depth() -> None:
    """The default accepts depth 1000 and rejects the next edge."""
    accepted = filter_frames(_cause_chain(_DEFAULT_MAX_EXCEPTION_DEPTH + 1))
    assert _walk_cause_chain(accepted) == _DEFAULT_MAX_EXCEPTION_DEPTH + 1, (
        "the default depth must include a node exactly at depth 1000"
    )

    with pytest.raises(ValueError, match=r"max_exception_depth=1000"):
        filter_frames(_cause_chain(_DEFAULT_MAX_EXCEPTION_DEPTH + 2))


def test_filter_frames_handles_one_hundred_thousand_nested_payloads() -> None:
    """Raised graph limits permit deep input without native or Python recursion."""
    node_count = 100_000
    input_nodes: list[ExceptionPayload | None] = []
    payload = _cause_chain(node_count, keepalive=input_nodes)
    filtered = filter_frames(
        payload,
        max_exception_depth=node_count - 1,
        max_exception_nodes=node_count,
    )

    output_nodes: list[FilteredPayload | None] = []
    current: FilteredPayload | None = filtered
    while current is not None:
        output_nodes.append(current)
        current = current.get("cause")

    assert sum(node is not None for node in output_nodes) == node_count, (
        "iterative traversal must return every node in a 100,000-node chain"
    )

    # Keep each graph node alive while releasing its root so CPython decrefs
    # the long chains iteratively rather than nesting dictionary destruction.
    del payload
    del filtered
    for index, _node in zip(range(len(input_nodes)), input_nodes, strict=True):
        input_nodes[index] = None
    for index, _node in zip(range(len(output_nodes)), output_nodes, strict=True):
        output_nodes[index] = None


def _wide_nested_groups(group_width: int) -> ExceptionPayload:
    """Build two exception groups with the requested member count each."""
    root = make_exception_payload([], type_name="ExceptionGroup")
    root["exceptions"] = [
        make_exception_payload(
            [],
            type_name="ExceptionGroup",
            message=f"group {group_index}",
        )
        for group_index in range(2)
    ]
    for group in root["exceptions"]:
        group["exceptions"] = [
            make_exception_payload([], type_name="ValueError")
            for _ in range(group_width)
        ]
    return root


def test_filter_frames_bounds_wide_nested_groups_by_default() -> None:
    """Nested group widths share the default root-inclusive node budget."""
    with pytest.raises(ValueError, match=r"max_exception_nodes=10000"):
        filter_frames(_wide_nested_groups(6_000))


def test_filter_frames_accepts_wide_nested_groups_with_raised_budget() -> None:
    """A wide nested group succeeds when depth and occurrence budgets fit."""
    expected_nodes = 1 + 2 + (2 * 6_000)
    filtered = filter_frames(
        _wide_nested_groups(6_000),
        max_exception_depth=2,
        max_exception_nodes=expected_nodes,
    )

    assert _walk_exception_graph(filtered) == expected_nodes, (
        "the raised budget must accept the complete nested group graph"
    )


def test_filter_frames_counts_shared_dag_expansion_per_occurrence() -> None:
    """Repeated subgraphs cannot multiply work beyond the default node budget."""
    shared = make_exception_payload([], type_name="ExceptionGroup")
    shared["exceptions"] = [
        make_exception_payload([], type_name="ValueError") for _ in range(6_000)
    ]
    root = make_exception_payload([])
    raw_root = typ.cast("dict[str, object]", root)
    raw_root["cause"] = shared
    raw_root["context"] = shared

    with pytest.raises(ValueError, match=r"max_exception_nodes=10000"):
        filter_frames(root)


def test_filter_frames_raises_for_invalid_graph_limits() -> None:
    """Zero and out-of-range graph limits retain their documented errors."""
    payload = make_exception_payload([])
    with pytest.raises(ValueError, match=r"max_exception_nodes=0"):
        filter_frames(payload, max_exception_nodes=0)

    with pytest.raises(OverflowError):
        filter_frames(payload, max_exception_depth=-1)

    with pytest.raises(OverflowError):
        filter_frames(payload, max_exception_nodes=-1)

    with pytest.raises(OverflowError):
        filter_frames(payload, max_exception_depth=2**100)


def test_stack_payload_ignores_exception_graph_limits() -> None:
    """Graph limits do not change filtering of a non-exception stack payload."""
    filtered = filter_frames(
        make_stack_payload(["app.py"]),
        max_exception_depth=0,
        max_exception_nodes=0,
    )

    assert [frame["filename"] for frame in filtered["frames"]] == ["app.py"], (
        "exception-graph limits must not affect stack-payload filtering"
    )
