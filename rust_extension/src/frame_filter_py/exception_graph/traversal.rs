//! Work-stack operations for exception payload graph traversal.

use std::collections::HashSet;

use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use super::super::filter_payload_frames;
use super::{
    ActiveNode, AttachTarget, ChildOccurrence, DictEdgeWork, Edge, EnterNode,
    ExceptionGraphTraversal, GroupWork, TraversalContext, Work, attach_filtered_payload,
    cycle_path, depth_limit_error, node_limit_error,
};

impl<'py, 'filters> ExceptionGraphTraversal<'py, 'filters> {
    /// Start a walk at the root payload, whose graph depth is zero.
    pub(super) fn new(
        payload: &Bound<'py, PyDict>,
        context: TraversalContext<'py, 'filters>,
    ) -> Self {
        let root = EnterNode {
            payload: payload.as_any().clone(),
            depth: 0,
            edge: Edge::Root,
            target: AttachTarget::Root,
        };
        Self {
            context,
            work: vec![Work::Enter(root)],
            pending_enters: 1,
            active_identities: HashSet::new(),
            active_path: Vec::new(),
            visited_nodes: 0,
            filtered_root: None,
        }
    }

    /// Process pending work until every reachable exception payload is copied.
    pub(super) fn run(mut self) -> PyResult<Bound<'py, PyDict>> {
        while let Some(task) = self.work.pop() {
            self.process_task(task)?;
        }

        self.filtered_root.take().ok_or_else(|| {
            PyRuntimeError::new_err("exception payload traversal did not produce a root")
        })
    }

    /// Dispatch one work item without nesting it inside the traversal loop.
    fn process_task(&mut self, task: Work<'py>) -> PyResult<()> {
        match task {
            Work::Enter(enter) => {
                self.consume_pending_enter()?;
                self.enter_node(enter)
            }
            Work::FilterGroup(group) => self.filter_group(group),
            Work::Leave { identity, .. } => self.leave_node(identity),
        }
    }

    /// Filter and attach one payload, then schedule its cause and context.
    fn enter_node(&mut self, enter: EnterNode<'py>) -> PyResult<()> {
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
        self.check_depth(depth)?;
        self.check_node_budget()?;
        self.visited_nodes += 1;

        let filtered = filter_payload_frames(self.context.py, &node, &self.context.filter_options)?;
        attach_filtered_payload(target, filtered.clone(), &mut self.filtered_root)?;
        self.active_identities.insert(identity);
        self.active_path.push(ActiveNode { identity, edge });
        self.work.push(Work::Leave {
            identity,
            _payload: node.clone(),
        });
        self.work.push(Work::FilterGroup(GroupWork {
            payload: node.clone(),
            result: filtered.clone(),
            depth,
        }));
        self.schedule_exception_edges(&node, &filtered, depth)?;
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
    fn check_depth(&self, depth: usize) -> PyResult<()> {
        if depth > self.context.limits.max_depth {
            return Err(depth_limit_error(self.context.limits.max_depth));
        }
        Ok(())
    }

    /// Enforce the root-inclusive count of entered payload occurrences.
    fn check_node_budget(&self) -> PyResult<()> {
        if self.visited_nodes >= self.context.limits.max_nodes {
            return Err(node_limit_error(self.context.limits.max_nodes));
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

    /// Schedule cause and context in reverse stack order for established traversal.
    fn schedule_exception_edges(
        &mut self,
        node: &Bound<'py, PyDict>,
        filtered: &Bound<'py, PyDict>,
        depth: usize,
    ) -> PyResult<()> {
        self.schedule_dict_edge(DictEdgeWork {
            payload: node.clone(),
            result: filtered.clone(),
            depth,
            edge: Edge::Context,
        })?;
        self.schedule_dict_edge(DictEdgeWork {
            payload: node.clone(),
            result: filtered.clone(),
            depth,
            edge: Edge::Cause,
        })
    }

    /// Inspect a cause or context edge and schedule its child occurrence.
    fn schedule_dict_edge(&mut self, work: DictEdgeWork<'py>) -> PyResult<()> {
        let key = work.edge.dictionary_key().ok_or_else(|| {
            PyRuntimeError::new_err("exception traversal expected a dictionary edge")
        })?;
        if let Some(payload) = work.payload.get_item(key)? {
            self.schedule_child(ChildOccurrence {
                payload,
                parent_depth: work.depth,
                edge: work.edge,
                target: AttachTarget::DictItem {
                    parent: work.result,
                    key,
                },
            });
        }
        Ok(())
    }

    /// Convert a child occurrence into the work-stack entry used by the walk.
    fn schedule_child(&mut self, child: ChildOccurrence<'py>) {
        self.schedule_enter(EnterNode {
            payload: child.payload,
            depth: child.parent_depth.saturating_add(1),
            edge: child.edge,
            target: child.target,
        });
    }

    /// Validate and schedule the members of an exception group.
    fn filter_group(&mut self, group: GroupWork<'py>) -> PyResult<()> {
        let Some(exceptions) = group.payload.get_item("exceptions")? else {
            return Ok(());
        };
        let exceptions = exceptions
            .cast::<PyList>()
            .map_err(|_| PyTypeError::new_err("'exceptions' must be a list"))?;
        let exception_count = exceptions.len();
        self.check_group_width(exception_count)?;

        // The list is bounded before allocating its placeholder slots.
        let filtered_exceptions = PyList::empty(self.context.py);
        for _ in 0..exception_count {
            filtered_exceptions.append(self.context.py.None())?;
        }
        group.result.set_item("exceptions", &filtered_exceptions)?;
        self.schedule_group_members(exceptions, &filtered_exceptions, group.depth)?;
        Ok(())
    }

    /// Ensure all group members fit the remaining occurrence budget.
    fn check_group_width(&self, exception_count: usize) -> PyResult<()> {
        let accounted_nodes = self.visited_nodes.saturating_add(self.pending_enters);
        let remaining_nodes = self
            .context
            .limits
            .max_nodes
            .saturating_sub(accounted_nodes);
        if exception_count > remaining_nodes {
            return Err(node_limit_error(self.context.limits.max_nodes));
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
            self.schedule_child(ChildOccurrence {
                payload: exceptions.get_item(index)?,
                parent_depth: depth,
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
