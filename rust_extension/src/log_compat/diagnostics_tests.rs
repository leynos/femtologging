//! Tests for warning visibility at the Rust log bridge boundary.

use super::diagnostics::mirror_ignored_warning;

#[test]
fn ignored_internal_warnings_are_mirrored_to_stderr() {
    let target = format!("{}::logger::producer::admission", env!("CARGO_CRATE_NAME"));
    let warning = log::Record::builder()
        .args(format_args!("dispatch capacity exhausted"))
        .level(log::Level::Warn)
        .target(&target)
        .build();
    let internal_info = log::Record::builder()
        .args(format_args!("ordinary internal record"))
        .level(log::Level::Info)
        .target(&target)
        .build();
    let application_warning = log::Record::builder()
        .args(format_args!("application warning"))
        .level(log::Level::Warn)
        .target("application")
        .build();
    let mut diagnostics = Vec::new();

    mirror_ignored_warning(&warning, |message| {
        diagnostics.push(message.to_string());
    });
    mirror_ignored_warning(&internal_info, |message| {
        diagnostics.push(message.to_string());
    });
    mirror_ignored_warning(&application_warning, |message| {
        diagnostics.push(message.to_string());
    });

    assert_eq!(diagnostics, ["dispatch capacity exhausted"]);
}
