//! How deep into an exception graph a node sits.
//!
//! The walker keeps two independent counters because the two ceilings bound
//! different edges: cause and context links consume
//! [`super::limits::CaptureLimits::max_chain_depth`], while descending into an
//! exception group consumes
//! [`super::limits::CaptureLimits::max_group_depth`]. A node reached through
//! both has spent one of each.
//!
//! The position is a small `Copy` value passed down the recursion rather than
//! mutable state on the walker, so a sibling branch — the context after the
//! cause, or the next group member — starts from the same depth its parent had.

/// How deep into the graph the current node sits.
#[derive(Clone, Copy)]
pub(super) struct Recursion {
    /// Cause and context links already followed.
    pub(super) chain_depth: usize,
    /// Exception groups already descended into.
    pub(super) group_depth: usize,
    /// Whether this node is the exception capture was asked about.
    pub(super) is_root: bool,
}

impl Recursion {
    /// The position of the exception capture was asked about.
    pub(super) const fn root() -> Self {
        Self {
            chain_depth: 0,
            group_depth: 0,
            is_root: true,
        }
    }

    /// The position of an exception reached through a cause or context link.
    pub(super) const fn through_link(self) -> Self {
        Self {
            chain_depth: self.chain_depth + 1,
            group_depth: self.group_depth,
            is_root: false,
        }
    }

    /// The position of an exception reached as a group child.
    pub(super) const fn through_group(self) -> Self {
        Self {
            chain_depth: self.chain_depth,
            group_depth: self.group_depth + 1,
            is_root: false,
        }
    }
}
