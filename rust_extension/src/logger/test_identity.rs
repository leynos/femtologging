//! Opaque test-only identities for attached handlers and filters.
//!
//! Python tests compare a logger's attachment set before and after a
//! reconfiguration to prove that an operation retained, replaced or appended a
//! native attachment. Reporting native addresses would expose the process
//! layout through the FFI boundary and would make the hooks available to code
//! that is not a test. These tokens carry a weak reference instead: they
//! compare by allocation identity and expose neither an integer value nor an
//! address.
//!
//! The weak reference also makes the comparison stable. Once a handler is
//! dropped its allocation stays reserved for as long as a token refers to it,
//! so a later attachment cannot inherit the identity of a dropped one even if
//! the allocator would otherwise reuse the address.

use std::sync::Weak;

use pyo3::IntoPyObjectExt;
use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::PyAnyMethods;

use crate::filters::FemtoFilter;
use crate::handler::FemtoHandlerTrait;

/// The native attachment an [`AttachmentIdentity`] refers to.
///
/// Handlers and filters live in different collections, so a token records
/// which kind it identifies. Two tokens of different kinds never compare
/// equal, even in the unlikely event that their allocations share an address.
enum AttachmentRef {
    /// An attachment whose target receives log records.
    Handler(Weak<dyn FemtoHandlerTrait>),
    /// An attachment whose target accepts or enriches records in flight.
    Filter(Weak<dyn FemtoFilter>),
}

impl AttachmentRef {
    /// Return whether both references identify the same allocation.
    fn same_allocation(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Handler(left), Self::Handler(right)) => Weak::ptr_eq(left, right),
            (Self::Filter(left), Self::Filter(right)) => Weak::ptr_eq(left, right),
            (Self::Handler(_), Self::Filter(_)) | (Self::Filter(_), Self::Handler(_)) => false,
        }
    }
}

/// Opaque identity token for one handler or filter attached to a logger.
///
/// The token is frozen: nothing can retarget it after creation, and the Python
/// module never registers it, so only the test hooks can produce one. Tests
/// compare tokens with `==` or `!=`; every other operator and every foreign
/// value yields `NotImplemented`, leaving Python's usual fallback rules in
/// place.
///
/// # Examples
///
/// ```ignore
/// let first = logger.handler_identities_for_test(py)?;
/// let second = logger.handler_identities_for_test(py)?;
/// assert_eq!(first, second);
/// ```
#[pyclass(frozen)]
pub(crate) struct AttachmentIdentity {
    /// The attachment this token identifies.
    attachment: AttachmentRef,
}

impl AttachmentIdentity {
    /// Wrap an attached handler in an identity token.
    pub(crate) fn for_handler(handler: Weak<dyn FemtoHandlerTrait>) -> Self {
        Self {
            attachment: AttachmentRef::Handler(handler),
        }
    }

    /// Wrap an attached filter in an identity token.
    pub(crate) fn for_filter(filter: Weak<dyn FemtoFilter>) -> Self {
        Self {
            attachment: AttachmentRef::Filter(filter),
        }
    }
}

#[pymethods]
impl AttachmentIdentity {
    /// Compare this token with another token by allocation identity.
    ///
    /// Equality holds when both tokens refer to the same handler allocation or
    /// both refer to the same filter allocation. The method returns
    /// `NotImplemented` for a foreign value and for ordering operators, which
    /// tells Python to apply its own comparison rules instead of raising from
    /// inside the extension.
    fn __richcmp__(
        &self,
        other: &Bound<'_, PyAny>,
        op: CompareOp,
        py: Python<'_>,
    ) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(py.NotImplemented());
        };
        match op {
            CompareOp::Eq => self
                .attachment
                .same_allocation(&other.attachment)
                .into_py_any(py),
            CompareOp::Ne => (!self.attachment.same_allocation(&other.attachment)).into_py_any(py),
            CompareOp::Lt | CompareOp::Le | CompareOp::Gt | CompareOp::Ge => {
                Ok(py.NotImplemented())
            }
        }
    }

    /// Return a constant description that never reveals the native address.
    fn __repr__(&self) -> &'static str {
        "<femtologging.AttachmentIdentity>"
    }
}

#[cfg(test)]
mod tests {
    //! Unit coverage for allocation comparison and the Python comparison slot.

    use std::sync::Arc;

    use super::*;
    use crate::filters::{FemtoFilter, FilterContext, FilterDecision};
    use crate::log_record::FemtoLogRecord;
    use crate::test_utils::collecting_handler::CollectingHandler;

    /// Filter used only to produce a distinct filter allocation.
    struct AcceptAll;

    impl FemtoFilter for AcceptAll {
        fn decision(
            &self,
            _record: &mut FemtoLogRecord,
            _context: &mut FilterContext,
        ) -> FilterDecision {
            FilterDecision::accept(true)
        }
    }

    fn handler_ref(handler: &Arc<dyn FemtoHandlerTrait>) -> AttachmentRef {
        AttachmentRef::Handler(Arc::downgrade(handler))
    }

    fn filter_ref(filter: &Arc<dyn FemtoFilter>) -> AttachmentRef {
        AttachmentRef::Filter(Arc::downgrade(filter))
    }

    #[test]
    fn same_allocation_is_reflexive_and_symmetric() {
        let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(CollectingHandler::new());
        let other: Arc<dyn FemtoHandlerTrait> = Arc::new(CollectingHandler::new());
        let first = handler_ref(&handler);
        let again = handler_ref(&handler);
        let distinct = handler_ref(&other);

        assert!(
            first.same_allocation(&first),
            "an allocation matches itself"
        );
        assert!(
            first.same_allocation(&again),
            "two weak refs match one allocation"
        );
        assert!(again.same_allocation(&first), "matching must be symmetric");
        assert!(
            !first.same_allocation(&distinct),
            "distinct handler allocations must not match"
        );
    }

    #[test]
    fn handler_and_filter_allocations_never_match() {
        let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(CollectingHandler::new());
        let filter: Arc<dyn FemtoFilter> = Arc::new(AcceptAll);
        let handler_ref = handler_ref(&handler);
        let filter_ref = filter_ref(&filter);

        assert!(
            !handler_ref.same_allocation(&filter_ref),
            "a handler must never match a filter"
        );
        assert!(
            !filter_ref.same_allocation(&handler_ref),
            "a filter must never match a handler"
        );
    }

    /// Build the comparison tokens used by the `__richcmp__` cases.
    fn comparison_tokens(
        py: Python<'_>,
    ) -> (
        Py<AttachmentIdentity>,
        Py<AttachmentIdentity>,
        Py<AttachmentIdentity>,
        Py<AttachmentIdentity>,
    ) {
        let first: Arc<dyn FemtoHandlerTrait> = Arc::new(CollectingHandler::new());
        let second: Arc<dyn FemtoHandlerTrait> = Arc::new(CollectingHandler::new());
        let filter: Arc<dyn FemtoFilter> = Arc::new(AcceptAll);
        let token = Py::new(py, AttachmentIdentity::for_handler(Arc::downgrade(&first)))
            .expect("the first token must be created");
        let same = Py::new(py, AttachmentIdentity::for_handler(Arc::downgrade(&first)))
            .expect("the second token must be created");
        let other = Py::new(py, AttachmentIdentity::for_handler(Arc::downgrade(&second)))
            .expect("the distinct handler token must be created");
        let filter_token = Py::new(py, AttachmentIdentity::for_filter(Arc::downgrade(&filter)))
            .expect("the filter token must be created");
        (token, same, other, filter_token)
    }

    /// Compare `left` with `right` under `op`, as Python's slot receives them.
    fn compare(
        py: Python<'_>,
        left: &Py<AttachmentIdentity>,
        right: &Bound<'_, PyAny>,
        op: CompareOp,
    ) -> Py<PyAny> {
        left.borrow(py)
            .__richcmp__(right, op, py)
            .expect("comparison must not fail")
    }

    /// Report whether `value` is the `NotImplemented` sentinel.
    fn is_not_implemented(py: Python<'_>, value: &Py<PyAny>) -> bool {
        value.bind(py).is(py.NotImplemented())
    }

    /// Extract a Python truth value from a comparison result.
    fn is_true(py: Python<'_>, value: &Py<PyAny>) -> bool {
        value
            .bind(py)
            .extract::<bool>()
            .expect("an equality result must be a bool")
    }

    #[test]
    fn richcmp_compares_allocations_and_defers_otherwise() {
        Python::attach(|py| {
            let (token, same, other, filter_token) = comparison_tokens(py);

            assert!(
                is_true(py, &compare(py, &token, same.bind(py), CompareOp::Eq)),
                "two tokens for one allocation must compare equal"
            );
            assert!(
                !is_true(py, &compare(py, &token, other.bind(py), CompareOp::Eq)),
                "tokens for distinct attachments must not compare equal"
            );
            assert!(
                is_true(py, &compare(py, &token, other.bind(py), CompareOp::Ne)),
                "tokens for distinct attachments must compare unequal"
            );
            assert!(
                !is_true(
                    py,
                    &compare(py, &token, filter_token.bind(py), CompareOp::Eq)
                ),
                "a handler token must not equal a filter token"
            );
        });
    }

    #[test]
    fn richcmp_defers_foreign_operands_and_ordering() {
        Python::attach(|py| {
            let (token, same, _, _) = comparison_tokens(py);
            let foreign = py.None();

            assert!(
                is_not_implemented(py, &compare(py, &token, foreign.bind(py), CompareOp::Eq)),
                "a foreign operand must defer to Python"
            );
            for op in [CompareOp::Lt, CompareOp::Le, CompareOp::Gt, CompareOp::Ge] {
                assert!(
                    is_not_implemented(py, &compare(py, &token, same.bind(py), op)),
                    "ordering operators must defer to Python"
                );
            }
        });
    }
}
