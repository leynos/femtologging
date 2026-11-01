"""Generated tests for bounded exception-payload graph traversal."""

from __future__ import annotations

import typing as typ

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from femtologging import filter_frames

from .conftest import ExceptionPayload, FilteredPayload, make_exception_payload
from .exception_graph_builders import (
    _EDGE_KEYS,
    _build_acyclic_graph,
    _build_cyclic_chain,
    _GraphPlan,
    _NodePlan,
)

type _AcyclicCase = tuple[_GraphPlan, tuple[str, ...]]
type _ChildOccurrence = tuple[FilteredPayload, int, int]


@st.composite
def _acyclic_graph_cases(draw: st.DrawFn) -> _AcyclicCase:
    """Generate small DAGs with varied edges, sharing and key insertion order."""
    node_count = draw(st.integers(min_value=2, max_value=5))
    plan: list[_NodePlan] = []

    for parent in range(node_count):
        children = tuple(range(parent + 1, node_count))
        if not children:
            plan.append((None, None, ()))
            continue

        if parent == 0:
            cause = draw(st.sampled_from(children))
            context = cause
        else:
            cause = draw(st.one_of(st.none(), st.sampled_from(children)))
            context = draw(st.one_of(st.none(), st.sampled_from(children)))
        members = draw(st.lists(st.sampled_from(children), max_size=2))
        plan.append((cause, context, tuple(members)))

    insertion_order = draw(st.permutations(_EDGE_KEYS))
    return tuple(plan), tuple(insertion_order)


def _children(plan: _GraphPlan, node_id: int) -> list[int]:
    """Return one node's child occurrences in the traversal's edge order."""
    cause, context, members = plan[node_id]
    children = [child for child in (cause, context) if child is not None]
    children.extend(members)
    return children


def _expanded_graph(plan: _GraphPlan) -> list[tuple[int, int]]:
    """List the depth-first expansion, counting every shared occurrence."""
    pending = [(0, 0)]
    occurrences: list[tuple[int, int]] = []
    while pending:
        node_id, depth = pending.pop()
        occurrences.append((node_id, depth))
        pending.extend(
            (child, depth + 1) for child in reversed(_children(plan, node_id))
        )
    return occurrences


def _snapshot_node(
    node: ExceptionPayload,
) -> tuple[tuple[object, ...], list[ExceptionPayload]]:
    """Capture one payload's contents and return its nested payloads."""
    cause = node.get("cause")
    context = node.get("context")
    frames = node.get("frames", [])
    exceptions = node.get("exceptions", [])
    snapshot = (
        id(node),
        tuple(node.keys()),
        node["type_name"],
        node["message"],
        id(frames) if "frames" in node else None,
        tuple(tuple(frame.items()) for frame in frames),
        id(cause) if cause is not None else None,
        id(context) if context is not None else None,
        id(exceptions) if "exceptions" in node else None,
        tuple(id(member) for member in exceptions),
    )
    nested = [child for child in (cause, context) if child is not None]
    nested.extend(exceptions)
    return snapshot, nested


def _snapshot_graph(root: ExceptionPayload) -> tuple[tuple[object, ...], ...]:
    """Capture node contents and edge identities without recursing."""
    pending = [root]
    seen: set[int] = set()
    snapshot: list[tuple[object, ...]] = []

    while pending:
        node = pending.pop()
        identity = id(node)
        if identity in seen:
            continue
        seen.add(identity)
        entry, nested = _snapshot_node(node)
        snapshot.append(entry)
        pending.extend(reversed(nested))

    return tuple(snapshot)


def _assert_payload_identity(
    payload: FilteredPayload,
    input_identities: set[int],
    output_identities: set[int],
) -> None:
    """Assert output occurrences are distinct copies of input payloads."""
    identity = id(payload)
    assert identity not in output_identities, f"output payload {identity} is repeated"
    assert identity not in input_identities, (
        f"output payload {identity} aliases an input"
    )
    output_identities.add(identity)


def _dict_child(
    payload: FilteredPayload,
    edge: str,
    child_id: int | None,
    depth: int,
) -> _ChildOccurrence | None:
    """Check one optional dictionary edge and return its output occurrence."""
    if child_id is None:
        assert edge not in payload, f"unexpected {edge} edge without a child occurrence"
        return None
    child = typ.cast("FilteredPayload", payload[edge])
    return child, child_id, depth + 1


def _filtered_children(
    payload: FilteredPayload,
    node_id: int,
    depth: int,
    plan: _GraphPlan,
) -> list[_ChildOccurrence]:
    """Check and list a payload's child outputs in traversal order."""
    cause, context, members = plan[node_id]
    children = [
        child
        for child in (
            _dict_child(payload, "cause", cause, depth),
            _dict_child(payload, "context", context, depth),
        )
        if child is not None
    ]
    if members:
        exceptions = typ.cast("list[FilteredPayload]", payload["exceptions"])
        assert len(exceptions) == len(members), (
            f"expected {len(members)} exception outputs, got {len(exceptions)}"
        )
        children.extend(
            (member, child_id, depth + 1)
            for member, child_id in zip(exceptions, members, strict=True)
        )
    else:
        assert "exceptions" not in payload, "payload has unplanned exception members"
    return children


def _assert_filtered_graph(
    root: FilteredPayload,
    plan: _GraphPlan,
    input_identities: set[int],
) -> list[tuple[int, int]]:
    """Check attachment order, occurrence count and independent output copies."""
    pending: list[_ChildOccurrence] = [(root, 0, 0)]
    output_identities: set[int] = set()
    observed: list[tuple[int, int]] = []

    while pending:
        payload, node_id, depth = pending.pop()
        _assert_payload_identity(payload, input_identities, output_identities)
        assert payload["type_name"] == f"GraphNode{node_id}", (
            f"payload at node {node_id} has type {payload['type_name']!r}"
        )
        assert "frames" not in payload, f"filtered node {node_id} retained frames"
        observed.append((node_id, depth))
        pending.extend(reversed(_filtered_children(payload, node_id, depth, plan)))

    return observed


@settings(max_examples=40, deadline=None)
@given(graph_case=_acyclic_graph_cases())
def test_generated_exception_dags_preserve_occurrences_and_limits(
    graph_case: _AcyclicCase,
) -> None:
    """Generated DAGs preserve edge order, copies and exact graph limits."""
    plan, insertion_order = graph_case
    root = _build_acyclic_graph(plan, insertion_order)
    before = _snapshot_graph(root)
    input_identities = {typ.cast("int", row[0]) for row in before}
    occurrences = _expanded_graph(plan)
    expected_count = len(occurrences)
    expected_depth = max(depth for _, depth in occurrences)

    filtered = typ.cast(
        "FilteredPayload",
        filter_frames(
            root,
            max_exception_depth=expected_depth,
            max_exception_nodes=expected_count,
        ),
    )

    observed = _assert_filtered_graph(filtered, plan, input_identities)
    assert observed == occurrences, "filtered traversal differs from the planned DAG"
    assert _snapshot_graph(root) == before, (
        "successful filtering mutated the input graph"
    )

    with pytest.raises(ValueError, match=f"max_exception_nodes={expected_count - 1}"):
        filter_frames(
            root,
            max_exception_depth=expected_depth,
            max_exception_nodes=expected_count - 1,
        )
    assert _snapshot_graph(root) == before, (
        "node-budget rejection mutated the input graph"
    )

    with pytest.raises(ValueError, match=f"max_exception_depth={expected_depth - 1}"):
        filter_frames(
            root,
            max_exception_depth=expected_depth - 1,
            max_exception_nodes=expected_count,
        )
    assert _snapshot_graph(root) == before, (
        "depth-limit rejection mutated the input graph"
    )


def _build_graph_with_selected_back_edges(
    cycle_edges: frozenset[str],
    insertion_order: tuple[str, ...],
) -> ExceptionPayload:
    """Build cause, context and group edges with selected active-path cycles."""
    root = make_exception_payload([], type_name="Root")
    cause_child = make_exception_payload([], type_name="CauseChild")
    context_child = make_exception_payload([], type_name="ContextChild")
    group_child = make_exception_payload([], type_name="GroupChild")
    for edge, child in (
        ("cause", cause_child),
        ("context", context_child),
        ("exceptions", group_child),
    ):
        if edge in cycle_edges:
            typ.cast("dict[str, object]", child)["cause"] = root

    root_edges: dict[str, object] = {
        "cause": cause_child,
        "context": context_child,
        "exceptions": [group_child],
    }
    for key in insertion_order:
        typ.cast("dict[str, object]", root)[key] = root_edges[key]
    return root


@settings(max_examples=30, deadline=None)
@given(
    node_count=st.integers(min_value=2, max_value=5),
    closing_edge=st.sampled_from(_EDGE_KEYS),
    insertion_order=st.permutations(_EDGE_KEYS),
    target=st.integers(min_value=0, max_value=4),
)
def test_generated_active_path_cycles_raise_without_mutation(
    node_count: int,
    closing_edge: str,
    insertion_order: tuple[str, ...],
    target: int,
) -> None:
    """Generated back-edges raise a cycle error and leave inputs unchanged."""
    cycle_target = target % node_count
    root = _build_cyclic_chain(node_count, cycle_target, closing_edge, insertion_order)
    before = _snapshot_graph(root)

    with pytest.raises(ValueError, match="exception payload graph contains a cycle at"):
        filter_frames(
            root,
            max_exception_depth=node_count + 1,
            max_exception_nodes=node_count + 2,
        )

    assert _snapshot_graph(root) == before, "cycle rejection mutated the input graph"


@settings(max_examples=30, deadline=None)
@given(
    cycle_edges=st.frozensets(st.sampled_from(_EDGE_KEYS), min_size=1),
    insertion_order=st.permutations(_EDGE_KEYS),
)
def test_generated_cycles_follow_edge_order_not_dictionary_order(
    cycle_edges: frozenset[str],
    insertion_order: tuple[str, ...],
) -> None:
    """The first active-path cycle follows cause/context/group traversal order."""
    root = _build_graph_with_selected_back_edges(cycle_edges, insertion_order)
    before = _snapshot_graph(root)
    first_cycle = next(edge for edge in _EDGE_KEYS if edge in cycle_edges)
    cycle_paths = {
        "cause": "$.cause.cause",
        "context": "$.context.cause",
        "exceptions": "$.exceptions[0].cause",
    }

    with pytest.raises(
        ValueError, match="exception payload graph contains a cycle at"
    ) as error:
        filter_frames(root, max_exception_depth=3, max_exception_nodes=20)

    assert (
        f"exception payload graph contains a cycle at {cycle_paths[first_cycle]}"
        in str(error.value)
    ), f"cycle path did not follow edge order for {first_cycle}"
    assert _snapshot_graph(root) == before, "cycle rejection mutated the input graph"
