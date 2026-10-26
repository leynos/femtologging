//! Work-stack operations for exception payload graph traversal.

use std::collections::HashSet;

use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use super::super::{ExceptionGraphLimits, FilterOptions, filter_exception_payload_node};
use super::{
    ActiveNode, AttachTarget, Edge, EnterNode, ExceptionGraphTraversal, Work,
    attach_filtered_payload, cycle_path, depth_limit_error, node_limit_error,
};

impl<'py> ExceptionGraphTraversal<'py> {
    /// Start a walk at the root payload, whose graph depth is zero.
    pub(super) fn new(payload: &Bound<'py, PyDict>) -> Self {
        let root = EnterNode {
            payload: payload.as_any().clone(),
            depth: 0,
            edge: Edge::Root,
            target: AttachTarget::Root,
        };
        Self {
            work: vec![Work::Enter(root)],
            pending_enters: 1,
            active_identities: HashSet::new(),
            active_path: Vec::new(),
            visited_nodes: 0,
            filtered_root: None,
        }
    }

    /// Process pending work until every reachable exception payload is copied.
    pub(super) fn run(
        mut self,
        py: Python<'py>,
        opts: &FilterOptions<'_>,
        limits: ExceptionGraphLimits,
    ) -> PyResult<Bound<'py, PyDict>> {
        while let Some(task) = self.work.pop() {
            self.process_task(py, task, opts, limits)?;
        }

        self.filtered_root.take().ok_or_else(|| {
            PyRuntimeError::new_err("exception payload traversal did not produce a root")
        })
    }

    /// Dispatch one work item without nesting it inside the traversal loop.
    fn process_task(
        &mut self,
        py: Python<'py>,
        task: Work<'py>,
        opts: &FilterOptions<'_>,
        limits: ExceptionGraphLimits,
    ) -> PyResult<()> {
        match task {
            Work::Enter(enter) => {
                self.consume_pending_enter()?;
                self.enter_node(py, enter, opts, limits)
            }
            Work::FilterGroup {
                payload,
                result,
                depth,
            } => self.filter_group(py, &payload, &result, depth, limits),
            Work::Leave { identity, .. } => self.leave_node(identity),
        }
    }

    /// Filter and attach one payload, then schedule its cause and context.
    fn enter_node(
        &mut self,
        py: Python<'py>,
        enter: EnterNode<'py>,
        opts: &FilterOptions<'_>,
        limits: ExceptionGraphLimits,
    ) -> PyResult<()> {
        let EnterNode {
            payload,
            depth,
            edge,
            target,
        } = enter;
        let node = payload
            .cast::<PyDict>()
            .map_err(|_| PyTypeError::new_err(edge.type_error_message()))?
            .clone();
        let identity = node.as_ptr() as usize;
        self.check_cycle(identity, edge)?;
        self.check_depth(depth, limits.max_depth)?;
        self.check_node_budget(limits.max_nodes)?;
        self.visited_nodes += 1;

        let filtered = filter_exception_payload_node(py, &node, opts)?;
        attach_filtered_payload(target, filtered.clone(), &mut self.filtered_root)?;
        self.active_identities.insert(identity);
        self.active_path.push(ActiveNode { identity, edge });
        self.work.push(Work::Leave {
            identity,
            _payload: node.clone(),
        });
        self.work.push(Work::FilterGroup {
            payload: node.clone(),
            result: filtered.clone(),
            depth,
        });
        self.schedule_context(&node, &filtered, depth)?;
        self.schedule_cause(&node, &filtered, depth)?;
        Ok(())
    }

    /// Reject a dictionary identity already present on this ancestor path.
    fn check_cycle(&self, identity: usize, edge: Edge) -> PyResult<()> {
        if self.active_identities.contains(&identity) {
            let path = cycle_path(&self.active_path, edge);
            return Err(PyValueError::new_err(format!(
                "exception payload graph contains a cycle at {path}"
            )));
        }
        Ok(())
    }

    /// Enforce the independent exception-graph depth limit.
    fn check_depth(&self, depth: usize, limit: usize) -> PyResult<()> {
        if depth > limit {
            return Err(depth_limit_error(limit));
        }
        Ok(())
    }

    /// Enforce the root-inclusive count of entered payload occurrences.
    fn check_node_budget(&self, limit: usize) -> PyResult<()> {
        if self.visited_nodes >= limit {
            return Err(node_limit_error(limit));
        }
        Ok(())
    }

    /// Remove one scheduled occurrence as it leaves the pending work stack.
    fn consume_pending_enter(&mut self) -> PyResult<()> {
        self.pending_enters = self.pending_enters.checked_sub(1).ok_or_else(|| {
            PyRuntimeError::new_err("exception payload traversal lost its pending-node count")
        })?;
        Ok(())
    }

    /// Schedule one payload occurrence and reserve it in the node budget.
    fn schedule_enter(&mut self, enter: EnterNode<'py>) {
        self.work.push(Work::Enter(enter));
        self.pending_enters += 1;
    }

    /// Schedule a present context edge after the cause edge is processed.
    fn schedule_context(
        &mut self,
        node: &Bound<'py, PyDict>,
        filtered: &Bound<'py, PyDict>,
        depth: usize,
    ) -> PyResult<()> {
        if let Some(context) = node.get_item("context")? {
            self.schedule_enter(EnterNode {
                payload: context,
                depth: depth.saturating_add(1),
                edge: Edge::Context,
                target: AttachTarget::DictItem {
                    parent: filtered.clone(),
                    key: "context",
                },
            });
        }
        Ok(())
    }

    /// Schedule a present cause edge so it is visited before context.
    fn schedule_cause(
        &mut self,
        node: &Bound<'py, PyDict>,
        filtered: &Bound<'py, PyDict>,
        depth: usize,
    ) -> PyResult<()> {
        if let Some(cause) = node.get_item("cause")? {
            self.schedule_enter(EnterNode {
                payload: cause,
                depth: depth.saturating_add(1),
                edge: Edge::Cause,
                target: AttachTarget::DictItem {
                    parent: filtered.clone(),
                    key: "cause",
                },
            });
        }
        Ok(())
    }

    /// Validate and schedule the members of an exception group.
    fn filter_group(
        &mut self,
        py: Python<'py>,
        payload: &Bound<'py, PyDict>,
        result: &Bound<'py, PyDict>,
        depth: usize,
        limits: ExceptionGraphLimits,
    ) -> PyResult<()> {
        let Some(exceptions) = payload.get_item("exceptions")? else {
            return Ok(());
        };
        let exceptions = exceptions
            .cast::<PyList>()
            .map_err(|_| PyTypeError::new_err("'exceptions' must be a list"))?;
        let exception_count = exceptions.len();
        self.check_group_width(exception_count, limits.max_nodes)?;

        // The list is bounded before allocating its placeholder slots.
        let filtered_exceptions = PyList::empty(py);
        for _ in 0..exception_count {
            filtered_exceptions.append(py.None())?;
        }
        result.set_item("exceptions", &filtered_exceptions)?;
        self.schedule_group_members(exceptions, &filtered_exceptions, depth)?;
        Ok(())
    }

    /// Ensure all group members fit the remaining occurrence budget.
    fn check_group_width(&self, exception_count: usize, limit: usize) -> PyResult<()> {
        let accounted_nodes = self.visited_nodes.saturating_add(self.pending_enters);
        let remaining_nodes = limit.saturating_sub(accounted_nodes);
        if exception_count > remaining_nodes {
            return Err(node_limit_error(limit));
        }
        Ok(())
    }

    /// Schedule group children in reverse so they execute in list order.
    fn schedule_group_members(
        &mut self,
        exceptions: &Bound<'py, PyList>,
        filtered: &Bound<'py, PyList>,
        depth: usize,
    ) -> PyResult<()> {
        for index in (0..exceptions.len()).rev() {
            self.schedule_enter(EnterNode {
                payload: exceptions.get_item(index)?,
                depth: depth.saturating_add(1),
                edge: Edge::Exception(index),
                target: AttachTarget::ListItem {
                    parent: filtered.clone(),
                    index,
                },
            });
        }
        Ok(())
    }

    /// Remove a completed node from the active ancestor path.
    fn leave_node(&mut self, identity: usize) -> PyResult<()> {
        let Some(active) = self.active_path.pop() else {
            return Err(PyRuntimeError::new_err(
                "exception payload traversal lost its active path",
            ));
        };
        self.check_leave_order(active.identity, identity)?;
        self.active_identities.remove(&identity);
        Ok(())
    }

    /// Ensure Leave work items are processed in the matching depth-first order.
    fn check_leave_order(&self, actual: usize, expected: usize) -> PyResult<()> {
        if actual != expected {
            return Err(PyRuntimeError::new_err(
                "exception payload traversal left nodes out of order",
            ));
        }
        Ok(())
    }
}
