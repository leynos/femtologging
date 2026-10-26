//! Iterative traversal of Python exception-payload graphs.
//!
//! This adapter-boundary traversal preserves each payload's shallow copy
//! semantics while keeping native stack use independent of graph depth.

use std::collections::HashSet;

#[path = "exception_graph/traversal.rs"]
mod traversal;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyList};

use super::{ExceptionGraphLimits, FilterOptions};

/// Filter an exception graph without using the native call stack for descent.
pub(super) fn filter_exception_payload<'py>(
    py: Python<'py>,
    payload: &Bound<'py, PyDict>,
    opts: &FilterOptions<'_>,
    limits: ExceptionGraphLimits,
) -> PyResult<Bound<'py, PyDict>> {
    ExceptionGraphTraversal::new(payload).run(py, opts, limits)
}

/// Attaches a filtered node to its root, parent dictionary, or group slot.
fn attach_filtered_payload<'py>(
    target: AttachTarget<'py>,
    filtered: Bound<'py, PyDict>,
    root: &mut Option<Bound<'py, PyDict>>,
) -> PyResult<()> {
    match target {
        AttachTarget::Root => *root = Some(filtered),
        AttachTarget::DictItem { parent, key } => parent.set_item(key, filtered)?,
        AttachTarget::ListItem { parent, index } => parent.set_item(index, filtered)?,
    }
    Ok(())
}

/// Renders the path from the root to an edge that closes a cycle.
fn cycle_path(active_path: &[ActiveNode], edge: Edge) -> String {
    let mut path = String::from("$");
    for active in active_path.iter().skip(1) {
        path.push_str(&active.edge.path_segment());
    }
    path.push_str(&edge.path_segment());
    path
}

/// Builds the error returned when a child exceeds the configured graph depth.
fn depth_limit_error(limit: usize) -> PyErr {
    PyValueError::new_err(format!(
        "exception payload graph exceeds max_exception_depth={limit}"
    ))
}

/// Builds the error returned when scheduled graph occurrences exceed the budget.
fn node_limit_error(limit: usize) -> PyErr {
    PyValueError::new_err(format!(
        "exception payload graph exceeds max_exception_nodes={limit}"
    ))
}

/// Carries one exception payload and its destination into the traversal stack.
struct EnterNode<'py> {
    /// Input payload to classify and filter.
    payload: Bound<'py, PyAny>,
    /// Number of exception edges from the root to this payload.
    depth: usize,
    /// Edge by which this payload is reached, used for errors and paths.
    edge: Edge,
    /// Location that receives the filtered copy of this payload.
    target: AttachTarget<'py>,
}

/// Non-recursive work items used to process exception payloads in edge order.
enum Work<'py> {
    /// Filters one payload and schedules its nested exception edges.
    Enter(EnterNode<'py>),
    /// Validates and schedules the members of an exception group.
    FilterGroup {
        /// Source payload whose group members are being processed.
        payload: Bound<'py, PyDict>,
        /// Filtered copy receiving the group members.
        result: Bound<'py, PyDict>,
        /// Graph depth of the group payload.
        depth: usize,
    },
    /// Removes a completed payload from the active ancestor path.
    Leave {
        /// Python object identity of the completed payload.
        identity: usize,
        /// Keeps the Python dictionary alive until its identity is removed.
        _payload: Bound<'py, PyDict>,
    },
}

/// Destination to which a filtered payload copy is attached.
enum AttachTarget<'py> {
    /// The root result returned to the caller.
    Root,
    /// A nested exception field on a parent payload.
    DictItem {
        /// Filtered parent dictionary receiving the child.
        parent: Bound<'py, PyDict>,
        /// Exception field name on the parent.
        key: &'static str,
    },
    /// One indexed member of a filtered exception group.
    ListItem {
        /// Filtered group list receiving the child.
        parent: Bound<'py, PyList>,
        /// Member index within the group list.
        index: usize,
    },
}

/// Exception-graph edge used to build paths and preserve malformed-edge errors.
#[derive(Clone, Copy)]
enum Edge {
    /// Root payload, which has no path segment.
    Root,
    /// The `cause` field of a payload.
    Cause,
    /// The `context` field of a payload.
    Context,
    /// A member at the given index of an exception group.
    Exception(usize),
}

impl Edge {
    /// Returns the path component contributed by this edge.
    fn path_segment(self) -> String {
        match self {
            Self::Root => String::new(),
            Self::Cause => ".cause".to_owned(),
            Self::Context => ".context".to_owned(),
            Self::Exception(index) => format!(".exceptions[{index}]"),
        }
    }

    /// Returns the established type-error text for a malformed edge.
    fn type_error_message(self) -> &'static str {
        match self {
            Self::Root => "exception payload root must be a dict",
            Self::Cause => "'cause' must be a dict",
            Self::Context => "'context' must be a dict",
            Self::Exception(_) => "each exception must be a dict",
        }
    }
}

/// Identity and incoming edge for one ancestor of the current traversal node.
struct ActiveNode {
    /// Python object identity used for cycle detection.
    identity: usize,
    /// Edge used to render paths from the root.
    edge: Edge,
}

/// Holds the mutable state of one iterative exception-graph walk.
struct ExceptionGraphTraversal<'py> {
    /// Pending work items, ordered so the next item preserves edge order.
    work: Vec<Work<'py>>,
    /// Scheduled `Enter` items that still reserve node-budget capacity.
    pending_enters: usize,
    /// Python dictionary identities on the active ancestor path.
    active_identities: HashSet<usize>,
    /// Incoming edges for rendering a cycle's path.
    active_path: Vec<ActiveNode>,
    /// Number of entered payload occurrences, including the root.
    visited_nodes: usize,
    /// Filtered copy of the root payload.
    filtered_root: Option<Bound<'py, PyDict>>,
}
