//! Frame-capture tests with injected ceilings.
//!
//! These tests ask what a frame window contains and how each frame field is
//! bounded. They live apart from [`super::traceback_capture_limits_tests`] so
//! both modules stay under the repository's module-size ceiling; the remaining
//! field and aggregate-budget tests stay in that module.

use pyo3::prelude::*;
use rstest::rstest;

use crate::test_utils::capture_limit_helpers::{capture_raised, marker, tiny_limits};
use crate::traceback_capture::CaptureLimits;

#[rstest]
fn source_indentation_is_trimmed_before_the_byte_ceiling() {
    Python::attach(|py| {
        // The raw line is far past the 32-byte ceiling, but the statement fits
        // once its indentation is removed, which is what a reader needs to see.
        let payload = capture_raised(
            py,
            c"
import pathlib, tempfile
root = pathlib.Path(tempfile.mkdtemp())
name = 'indented'
source = (
    'def ' + name + '():\\n'
    + ' ' * 600 + 'raise ValueError(\"sized\")\\n'
    + name + '()\\n'
)
target = root / 'indented_module.py'
target.write_text(source)
exec(compile(source, str(target), 'exec'), {'__name__': 'indented_module'})
",
            &tiny_limits(),
        );

        let frame = payload
            .frames
            .iter()
            .find(|frame| frame.function == "indented")
            .expect("the indented frame should be retained");
        assert_eq!(
            frame.source_line.as_deref(),
            Some("raise ValueError(\"sized\")"),
            "trimming must happen before the byte ceiling"
        );
    });
}

#[rstest]
fn a_traceback_past_the_visit_allowance_reports_a_lower_bound() {
    Python::attach(|py| {
        // Default limits allow 256 visits; this chain is longer, so the walk
        // stops before reaching the throw site. The marker must say the count
        // is a minimum rather than presenting the visited window as innermost.
        let payload = capture_raised(
            py,
            c"
def recurse(depth):
    if depth == 0:
        raise ValueError('deep')
    recurse(depth - 1)

recurse(300)
",
            &CaptureLimits::default(),
        );

        let marker = marker(&payload);
        assert!(
            marker.frames_omitted > 0,
            "the unvisited tail must be reported"
        );
        assert!(
            marker.frames_omitted_is_lower_bound,
            "a walk that stopped early cannot report an exact count: {marker:?}"
        );
    });
}

#[rstest]
fn frame_ceiling_keeps_the_innermost_frames() {
    Python::attach(|py| {
        // The chain is raised six calls deep so that the ceiling has to
        // discriminate: the frames kept are the last three, which are the ones
        // nearest the failure. A shallower chain would fit inside the ceiling
        // and prove nothing.
        let payload = capture_raised(
            py,
            c"
def level_four():
    level_three()

def level_three():
    level_two()

def level_two():
    level_one()

def level_one():
    innermost()

def innermost():
    raise ValueError('deep')

level_four()
",
            &tiny_limits(),
        );

        assert_eq!(payload.frames.len(), 3);
        let functions: Vec<&str> = payload.frames.iter().map(|f| f.function.as_str()).collect();
        assert_eq!(
            functions,
            vec!["level_two", "level_one", "innermost"],
            "the innermost frames should survive, in outermost-first order"
        );
        assert!(marker(&payload).frames_omitted > 0);
    });
}

#[rstest]
fn every_frame_field_respects_its_ceiling() {
    Python::attach(|py| {
        // The fixture needs a real file so `linecache` can supply a source
        // line, and every frame field must exceed its ceiling: a 300-character
        // function name, a path longer than the filename ceiling, and a raise
        // line padded past the source ceiling. Without all three, the test
        // would pass even if a clamp were missing.
        let payload = capture_raised(
            py,
            c"
import pathlib, tempfile
root = pathlib.Path(tempfile.mkdtemp())
path = root
for _ in range(4):
    path = path / ('d' * 120)
path.mkdir(parents=True)
name = 'f' * 300
source = (
    'def ' + name + '():\\n'
    '    raise ValueError(\"sized\")  # ' + 'x' * 600 + '\\n'
    + name + '()\\n'
)
target = path / 'long_module.py'
target.write_text(source)
exec(compile(source, str(target), 'exec'), {'__name__': 'long_module'})
",
            &tiny_limits(),
        );

        let long_frame = payload
            .frames
            .iter()
            .find(|frame| frame.function.starts_with('f'))
            .expect("the oversized frame should be retained");
        assert!(long_frame.filename.len() <= 24);
        assert!(long_frame.function.len() <= 24);
        let source = long_frame
            .source_line
            .as_ref()
            .expect("the long source line should be retained");
        assert!(
            source.len() <= 32,
            "source line of {} bytes exceeded the ceiling",
            source.len()
        );
        for frame in &payload.frames {
            // Capture never collects locals, so no frame pays for rendering a
            // binding's `__repr__`.
            assert!(frame.locals.is_none());
        }
    });
}
