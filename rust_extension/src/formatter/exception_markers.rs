//! Human-readable markers for bounded exception capture.
//!
//! Capture records every omission in [`PayloadTruncation`], but a reader needs
//! to see that record in the traceback text. The helpers here render the
//! markers that do not belong to a single structural element: dropped frames,
//! dropped cause/context links, and the summary of message, argument, note and
//! budget omissions.
//!
//! Every marker is bounded by the number of marker fields, never by the size of
//! the discarded data, so rendering a truncated megabyte message costs the same
//! as rendering a truncated short one.

use crate::exception_schema::PayloadTruncation;

/// Format marker lines for a cause or context that capture dropped entirely.
///
/// A link whose payload is displayed is not reported, because the marker exists
/// to explain absent text. Each link is decided on its own: a retained cause can
/// sit beside a dropped context, and the missing context still needs a marker.
pub(super) fn format_omitted_links(
    truncation: Option<&PayloadTruncation>,
    cause_displayed: bool,
    context_displayed: bool,
) -> String {
    let Some(marker) = truncation else {
        return String::new();
    };
    let mut output = String::new();
    if marker.cause_omitted && !cause_displayed {
        output.push_str("... exception cause omitted ...\n\n");
    }
    if marker.context_omitted && !context_displayed {
        output.push_str("... exception context omitted ...\n\n");
    }
    output
}

/// Render the frame-omission marker when capture dropped frames.
///
/// The count is a fixed-size integer, so the marker stays bounded however many
/// frames were left out.
pub(super) fn push_frame_omission(output: &mut String, truncation: Option<&PayloadTruncation>) {
    let Some(omitted) = truncation
        .map(|marker| marker.frames_omitted)
        .filter(|count| *count > 0)
    else {
        return;
    };
    output.push_str(&format!("... {omitted} earlier frames omitted ...\n"));
}

/// Format the summary line for omissions without a structural marker.
///
/// Frames, causes, contexts and group members already render their own
/// markers; this line covers message, argument, note, failure and whole-budget
/// omissions.
pub(super) fn format_exception_summary(truncation: Option<&PayloadTruncation>) -> String {
    let Some(marker) = truncation else {
        return String::new();
    };

    let mut details: Vec<String> = Vec::new();
    if marker.message_truncated {
        details.push("message truncated".to_owned());
    }
    push_count(&mut details, marker.args_omitted, "arguments omitted");
    if marker.args_truncated {
        details.push("argument representations truncated".to_owned());
    }
    push_count(&mut details, marker.notes_omitted, "notes omitted");
    if marker.notes_truncated {
        details.push("notes truncated".to_owned());
    }
    push_count(
        &mut details,
        marker.failed_elements,
        "elements omitted after conversion failures",
    );
    if marker.members_truncated {
        details.push("exception group members truncated".to_owned());
    }
    if marker.budget_exhausted {
        details.push("capture budget exhausted".to_owned());
    }

    if details.is_empty() {
        return String::new();
    }
    format!(
        "... exception payload truncated ({}) ...\n",
        details.join("; ")
    )
}

/// Append `"{count} {label}"` when `count` is positive.
fn push_count(details: &mut Vec<String>, count: u32, label: &str) {
    if count > 0 {
        details.push(format!("{count} {label}"));
    }
}
