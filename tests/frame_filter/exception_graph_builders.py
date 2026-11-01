"""Build generated exception payload graphs for property tests."""

from __future__ import annotations

import typing as typ

from .conftest import ExceptionPayload, make_exception_payload

type _NodePlan = tuple[int | None, int | None, tuple[int, ...]]
type _GraphPlan = tuple[_NodePlan, ...]

_EDGE_KEYS = ("cause", "context", "exceptions")


def _build_acyclic_graph(
    plan: _GraphPlan,
    insertion_order: tuple[str, ...],
) -> ExceptionPayload:
    """Build a Python payload DAG from forward-only node references."""
    nodes = [
        make_exception_payload([], type_name=f"GraphNode{index}")
        for index in range(len(plan))
    ]

    for index, node_plan in enumerate(plan):
        edges = _edges_for_plan(node_plan, nodes)
        _attach_edges(nodes[index], edges, insertion_order)

    return nodes[0]


def _edges_for_plan(
    node_plan: _NodePlan,
    nodes: list[ExceptionPayload],
) -> dict[str, object]:
    """Build the present exception edges for one generated node."""
    cause, context, members = node_plan
    edges: dict[str, object] = {
        key: nodes[child]
        for key, child in (("cause", cause), ("context", context))
        if child is not None
    }
    if members:
        edges["exceptions"] = [nodes[member] for member in members]
    return edges


def _attach_edges(
    node: ExceptionPayload,
    edges: dict[str, object],
    insertion_order: tuple[str, ...],
) -> None:
    """Attach present edge values in the generated insertion order."""
    node_dict = typ.cast("dict[str, object]", node)
    for key in insertion_order:
        if key in edges:
            node_dict[key] = edges[key]


def _build_cyclic_chain(
    node_count: int,
    target: int,
    closing_edge: str,
    insertion_order: tuple[str, ...],
) -> ExceptionPayload:
    """Build a cause chain closed by one generated back-edge."""
    nodes = [
        make_exception_payload([], type_name=f"CycleNode{index}")
        for index in range(node_count)
    ]
    for index, node in enumerate(nodes):
        edges: dict[str, object]
        if index + 1 < node_count:
            edges = {"cause": nodes[index + 1]}
        else:
            edges = {closing_edge: _closing_edge_target(closing_edge, nodes[target])}
        _attach_edges(node, edges, insertion_order)
    return nodes[0]


def _closing_edge_target(
    closing_edge: str,
    target: ExceptionPayload,
) -> object:
    """Wrap group back-edges in their required exception-member list."""
    if closing_edge == "exceptions":
        return [target]
    return target
