//! Compile percent format strings into literal text and field directives.
//!
//! Compilation happens once, when a [`PercentFormatter`] is constructed, so a
//! formatter never reparses its format string while rendering records. The
//! parser is linear in the format length: a malformed directive resumes
//! parsing one byte after its `%`, and the next closing-parenthesis search
//! reuses the result of the previous search instead of rescanning that suffix.
//!
//! [`PercentFormatter`]: super::PercentFormatter

/// Conversion applied to a directive's resolved field value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Conversion {
    /// `s`: render the field as a string.
    Str,
    /// `d`: render an integer, or nothing when the field is not an integer.
    Integer,
    /// `f`: render a fixed-point float, or nothing when the field is not numeric.
    Float,
}

impl Conversion {
    /// Return the conversion for `character`, or `None` when it is unsupported.
    pub(super) fn from_char(character: char) -> Option<Self> {
        match character {
            's' => Some(Self::Str),
            'd' => Some(Self::Integer),
            'f' => Some(Self::Float),
            _ => None,
        }
    }

    /// Render a resolved field value using this conversion.
    ///
    /// A missing or unparsable value for a numeric conversion renders as an
    /// empty string, matching the stdlib logging behaviour callers expect.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_eq!(Conversion::Integer.render(Some("12".to_owned())), "12");
    /// assert_eq!(Conversion::Integer.render(Some("twelve".to_owned())), "");
    /// ```
    pub(super) fn render(self, value: Option<String>) -> String {
        match self {
            Self::Str => value.unwrap_or_default(),
            Self::Integer => value
                .and_then(|value| value.parse::<i64>().ok())
                .map_or_else(String::new, |value| value.to_string()),
            Self::Float => value
                .and_then(|value| value.parse::<f64>().ok())
                .map_or_else(String::new, |value| format!("{value:.6}")),
        }
    }
}

/// One piece of a compiled format string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Segment {
    /// Text emitted verbatim, including a `%` that starts no valid directive.
    Literal(String),
    /// A record field rendered with a conversion while formatting.
    Directive {
        /// Field name between the parentheses of `%(<name>)<conversion>`.
        field: String,
        /// Conversion character that followed the closing parenthesis.
        conversion: Conversion,
    },
}

/// Compile `format` into literal text and directives in one linear pass.
///
/// An unsupported directive is preserved literally: the `%` that introduced it
/// is copied and parsing resumes at the next character. This keeps the output
/// identical to the documented per-record behaviour while examining each
/// position of the format string a bounded number of times.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(
///     compile("v=%(levelno)d 100%%"),
///     vec![
///         Segment::Literal("v=".to_owned()),
///         Segment::Directive {
///             field: "levelno".to_owned(),
///             conversion: Conversion::Integer,
///         },
///         Segment::Literal(" 100%".to_owned()),
///     ],
/// );
/// ```
pub(super) fn compile(format: &str) -> Vec<Segment> {
    #[cfg(test)]
    instrumentation::record_compilation();

    Compiler::new(format).run()
}

#[cfg(test)]
pub(crate) mod instrumentation {
    //! Test-only counters that observe how much work compilation performs.
    //!
    //! The formatter promises that a format is compiled once and that
    //! compilation is linear in its length, and neither promise is visible in
    //! rendered output. These counters make the work observable, so a
    //! regression fails a unit test instead of only a benchmark.
    //!
    //! Measurements are thread-local: each test's work belongs to the thread
    //! that performed it, which keeps the numbers deterministic under parallel
    //! test execution. The module is absent from production builds, so
    //! compilation there carries no instrumentation and no synchronisation.

    use std::cell::Cell;

    thread_local! {
        /// Work recorded on this thread since the last [`reset`].
        static MEASUREMENTS: Cell<Measurements> = const { Cell::new(Measurements::ZERO) };
    }

    /// Work observed while compiling format strings on one thread.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct Measurements {
        /// Compilations started.
        pub(crate) compilations: usize,
        /// Fresh closing-parenthesis scans performed.
        pub(crate) delimiter_searches: usize,
        /// Bytes examined by those fresh scans.
        pub(crate) scanned_bytes: usize,
    }

    impl Measurements {
        /// A measurement with no work recorded.
        const ZERO: Self = Self {
            compilations: 0,
            delimiter_searches: 0,
            scanned_bytes: 0,
        };
    }

    /// Clear this thread's measurements.
    pub(crate) fn reset() {
        MEASUREMENTS.with(|cell| cell.set(Measurements::ZERO));
    }

    /// Return this thread's measurements since the last reset.
    pub(crate) fn measurements() -> Measurements {
        MEASUREMENTS.with(Cell::get)
    }

    /// Record that one compilation started.
    pub(crate) fn record_compilation() {
        update(|measurements| measurements.compilations += 1);
    }

    /// Record one fresh closing-parenthesis scan of `scanned_bytes` bytes.
    pub(crate) fn record_delimiter_search(scanned_bytes: usize) {
        update(|measurements| {
            measurements.delimiter_searches += 1;
            measurements.scanned_bytes = measurements.scanned_bytes.saturating_add(scanned_bytes);
        });
    }

    /// Apply `change` to this thread's measurements.
    fn update(change: impl FnOnce(&mut Measurements)) {
        MEASUREMENTS.with(|cell| {
            let mut measurements = cell.get();
            change(&mut measurements);
            cell.set(measurements);
        });
    }
}

/// Working state for compiling one format string.
struct Compiler<'a> {
    /// Format text being compiled.
    format: &'a str,
    /// Literal text not yet flushed into `segments`.
    literal: String,
    /// Compiled segments, in output order.
    segments: Vec<Segment>,
    /// Search state reused across closing-parenthesis lookups.
    search: ClosingParenthesisSearch,
    /// Offset to resume scanning from.
    cursor: usize,
}

impl<'a> Compiler<'a> {
    /// Start compiling `format` with empty output.
    fn new(format: &'a str) -> Self {
        Self {
            format,
            literal: String::new(),
            segments: Vec::new(),
            search: ClosingParenthesisSearch::default(),
            cursor: 0,
        }
    }

    /// Compile the whole format, returning its segments in output order.
    fn run(mut self) -> Vec<Segment> {
        while let Some(relative) = self.format[self.cursor..].find('%') {
            let percent = self.cursor + relative;
            self.literal.push_str(&self.format[self.cursor..percent]);
            self.cursor = self.consume_percent(percent);
        }

        self.literal.push_str(&self.format[self.cursor..]);
        self.flush_literal();
        self.segments
    }

    /// Consume the `%` at `percent`, returning the offset to resume scanning from.
    fn consume_percent(&mut self, percent: usize) -> usize {
        let after_percent = percent + 1;

        // `%%` is an escaped percent sign, not a directive terminator.
        if self.format[after_percent..].starts_with('%') {
            self.literal.push('%');
            return after_percent + 1;
        }

        let Some((field, conversion, end)) = self.parse_directive(after_percent) else {
            self.literal.push('%');
            return after_percent;
        };

        self.flush_literal();
        self.segments.push(Segment::Directive {
            field: field.to_owned(),
            conversion,
        });
        end
    }

    /// Parse the directive whose opening parenthesis is at `open`.
    ///
    /// Returns the field name, the conversion, and the offset just past the
    /// conversion character when the directive is supported.
    fn parse_directive(&mut self, open: usize) -> Option<(&'a str, Conversion, usize)> {
        if !self.format[open..].starts_with('(') {
            return None;
        }

        let close = self.search.find(self.format, open + 1)?;
        let conversion_start = close + 1;
        let conversion_char = self.format[conversion_start..].chars().next()?;
        let conversion = Conversion::from_char(conversion_char)?;
        Some((
            &self.format[open + 1..close],
            conversion,
            conversion_start + conversion_char.len_utf8(),
        ))
    }

    /// Move buffered literal text into `segments` when it is non-empty.
    fn flush_literal(&mut self) {
        if !self.literal.is_empty() {
            self.segments
                .push(Segment::Literal(std::mem::take(&mut self.literal)));
        }
    }
}

/// Remembers the last closing-parenthesis search so parsing stays linear.
///
/// A malformed directive resumes one byte after its `%`, so the next attempt
/// searches a suffix of the text just searched. A later attempt whose search
/// start falls inside the previously searched range therefore sees the same
/// nearest `)`, and reusing that answer keeps the total work linear.
#[derive(Debug, Default)]
pub(super) struct ClosingParenthesisSearch {
    /// Offset the remembered result was found from, if any search has run.
    searched_from: Option<usize>,
    /// Offset of the nearest `)`, or `None` when the searched suffix holds none.
    close: Option<usize>,
}

impl ClosingParenthesisSearch {
    /// Return the offset of the nearest `)` at or after `from`.
    pub(super) fn find(&mut self, format: &str, from: usize) -> Option<usize> {
        if self.covers(from) {
            return self.close;
        }

        let suffix = &format[from..];
        let close = suffix.find(')').map(|relative| from + relative);
        #[cfg(test)]
        instrumentation::record_delimiter_search(
            close.map_or(suffix.len(), |close| close - from + 1),
        );
        self.searched_from = Some(from);
        self.close = close;
        close
    }

    /// Report whether the remembered result answers a search from `from`.
    ///
    /// A remembered `None` applies to every later offset because those offsets
    /// start suffixes of the already searched suffix.
    pub(super) fn covers(&self, from: usize) -> bool {
        match (self.searched_from, self.close) {
            (Some(searched_from), Some(close)) => from >= searched_from && from <= close,
            (Some(searched_from), None) => from >= searched_from,
            (None, _) => false,
        }
    }
}
