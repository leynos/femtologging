//! Identifier for registered formatters.
//!
//! Distinguishes between built-in formatter identifiers and arbitrary
//! user-provided IDs to avoid scattering string comparisons.

/// Maintains handler construction and delivery contracts so configuration is validated before asynchronous runtime state is published.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FormatterId {
    /// The built-in default formatter.
    Default,
    /// A user-specified formatter identifier.
    Custom(String),
}

impl FormatterId {
    /// Return the identifier as a string slice.
    pub const fn as_str(&self) -> &str {
        match self {
            Self::Default => "default",
            Self::Custom(id) => id.as_str(),
        }
    }
}

impl From<String> for FormatterId {
    fn from(id: String) -> Self {
        match id.as_str() {
            "default" => Self::Default,
            _ => Self::Custom(id),
        }
    }
}

impl From<&str> for FormatterId {
    fn from(id: &str) -> Self {
        Self::from(id.to_owned())
    }
}

#[cfg(test)]
mod const_eval_tests {
    //! Compile-time check for the formatter identifier's `const` accessor.
    //!
    //! `FormatterId` is crate-private, so an external `trybuild` fixture cannot
    //! name it — a UI fixture compiles as its own crate. This item is therefore
    //! the enforcing test for the const guarantee: it fails the build if
    //! `as_str` stops being const-evaluable, which a run-time test cannot
    //! detect. The public builder setters that consume this identifier are
    //! pinned externally by `tests/ui/pass/builder_const_setters.rs`.

    use super::FormatterId;

    // `as_str` must stay callable in const context.
    const _: &str = FormatterId::Default.as_str();
}
