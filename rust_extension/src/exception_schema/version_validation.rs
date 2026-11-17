//! Recursive schema version validation for [`ExceptionPayload`] trees.
//!
//! `cause`, `context`, and exception-group members are independently versioned
//! [`ExceptionPayload`] values, so checking only the root would leave nested
//! payloads unvalidated. [`validate_payload_tree`] walks the whole tree and
//! rejects the first unsupported `schema_version` it meets.
//!
//! The walk is depth-first in pre-order: each payload, then its `cause`, its
//! `context`, and its exception-group members in index order. That order makes
//! the reported failure the first one a reader of the payload would meet.
//!
//! A failure below the root is reported through
//! [`SchemaVersionError::Nested`] with a dot-separated path such as
//! `cause.exceptions[3].context`. The walk is bounded by
//! [`MAX_EXCEPTION_PAYLOAD_DEPTH`] and [`MAX_EXCEPTION_PAYLOAD_NODES`], which
//! bound validation work rather than parsing. Path segments are collected only
//! while an error unwinds, so successful validation allocates no path strings.

use super::{
    ExceptionPayload, MAX_EXCEPTION_PAYLOAD_DEPTH, MAX_EXCEPTION_PAYLOAD_NODES, SchemaVersionError,
    validate_schema_version,
};

/// One edge of the walk, used to describe where validation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathSegment {
    /// The `cause` link.
    Cause,
    /// The `context` link.
    Context,
    /// The exception-group member at the given index.
    Exception(usize),
}

/// Why the bounded walk stopped before it finished.
#[derive(Debug)]
enum Failure {
    /// A payload carried an unsupported schema version.
    Version {
        /// The version mismatch found at the end of `path`.
        error: SchemaVersionError,
        /// Segments from the root to the failing payload.
        path: Vec<PathSegment>,
    },
    /// The walk reached a payload deeper than the depth limit.
    TooDeep {
        /// Segments from the root to the payload past the limit.
        path: Vec<PathSegment>,
    },
    /// The walk visited more payloads than the node limit allows.
    TooLarge,
}

impl Failure {
    /// Convert the internal failure into the public error type.
    ///
    /// A root-level version mismatch is returned unchanged so existing callers
    /// keep observing a plain [`SchemaVersionError::VersionTooNew`] or
    /// [`SchemaVersionError::VersionTooOld`].
    fn into_error(self) -> SchemaVersionError {
        match self {
            Self::Version { error, path } if path.is_empty() => error,
            Self::Version { error, path } => SchemaVersionError::Nested {
                path: render_path(&path),
                source: Box::new(error),
            },
            Self::TooDeep { path } => SchemaVersionError::TreeTooDeep {
                path: render_path(&path),
                max_depth: MAX_EXCEPTION_PAYLOAD_DEPTH,
            },
            Self::TooLarge => SchemaVersionError::TreeTooLarge {
                max_nodes: MAX_EXCEPTION_PAYLOAD_NODES,
            },
        }
    }
}

/// Render dot-separated path segments, for example `cause.exceptions[3]`.
fn render_path(segments: &[PathSegment]) -> String {
    let mut path = String::new();
    for (position, segment) in segments.iter().enumerate() {
        if position > 0 {
            path.push('.');
        }
        push_segment(&mut path, *segment);
    }
    path
}

/// Append one segment to `path` without a fallible formatting call.
fn push_segment(path: &mut String, segment: PathSegment) {
    match segment {
        PathSegment::Cause => path.push_str("cause"),
        PathSegment::Context => path.push_str("context"),
        PathSegment::Exception(index) => {
            path.push_str("exceptions[");
            path.push_str(&index.to_string());
            path.push(']');
        }
    }
}

/// Validate `payload` and every payload reachable from it.
///
/// # Errors
///
/// See [`SchemaVersioned::validate_version`]; this is its implementation for
/// [`ExceptionPayload`].
///
/// [`SchemaVersioned::validate_version`]: super::SchemaVersioned::validate_version
pub(super) fn validate_payload_tree(payload: &ExceptionPayload) -> Result<(), SchemaVersionError> {
    let mut path = Vec::new();
    let mut visited = 0;
    visit(payload, 0, &mut visited, &mut path).map_err(Failure::into_error)
}

/// Visit `payload` and, unless a limit stops the walk, its descendants.
///
/// `depth` is the distance from the root, which is at depth 0. `visited`
/// counts payloads accepted into the walk so far, and `path` holds the
/// segments from the root to `payload`.
fn visit(
    payload: &ExceptionPayload,
    depth: usize,
    visited: &mut usize,
    path: &mut Vec<PathSegment>,
) -> Result<(), Failure> {
    if depth > MAX_EXCEPTION_PAYLOAD_DEPTH {
        return Err(Failure::TooDeep { path: path.clone() });
    }

    *visited += 1;
    if *visited > MAX_EXCEPTION_PAYLOAD_NODES {
        return Err(Failure::TooLarge);
    }

    validate_schema_version(payload.schema_version).map_err(|error| Failure::Version {
        error,
        path: path.clone(),
    })?;

    if let Some(cause) = payload.cause.as_deref() {
        visit_child(cause, PathSegment::Cause, depth, visited, path)?;
    }
    if let Some(context) = payload.context.as_deref() {
        visit_child(context, PathSegment::Context, depth, visited, path)?;
    }
    for (index, exception) in payload.exceptions.iter().enumerate() {
        visit_child(
            exception,
            PathSegment::Exception(index),
            depth,
            visited,
            path,
        )?;
    }

    Ok(())
}

/// Visit one child of the current payload, recording the edge in `path`.
///
/// The segment is popped again after a successful visit. A failed visit keeps
/// it in place: the failure has already captured the path that names the
/// offending payload, and the walk abandons `path` as it unwinds.
fn visit_child(
    child: &ExceptionPayload,
    segment: PathSegment,
    depth: usize,
    visited: &mut usize,
    path: &mut Vec<PathSegment>,
) -> Result<(), Failure> {
    path.push(segment);
    let outcome = visit(child, depth + 1, visited, path);
    if outcome.is_ok() {
        path.pop();
    }
    outcome
}
