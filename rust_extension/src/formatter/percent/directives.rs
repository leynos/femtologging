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
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut search = ClosingParenthesisSearch::default();
    let mut cursor = 0;

    while let Some(relative) = format[cursor..].find('%') {
        let percent = cursor + relative;
        literal.push_str(&format[cursor..percent]);
        cursor = consume_percent(format, percent, &mut literal, &mut segments, &mut search);
    }

    literal.push_str(&format[cursor..]);
    push_literal(&mut segments, &mut literal);
    segments
}

/// Consume the `%` at `percent`, returning the offset to resume scanning from.
///
/// The literal buffer is flushed into `segments` before a compiled directive
/// is pushed, so the two stay in output order.
fn consume_percent(
    format: &str,
    percent: usize,
    literal: &mut String,
    segments: &mut Vec<Segment>,
    search: &mut ClosingParenthesisSearch,
) -> usize {
    let after_percent = percent + 1;

    // `%%` is an escaped percent sign, not a directive terminator.
    if format[after_percent..].starts_with('%') {
        literal.push('%');
        return after_percent + 1;
    }

    let Some((field, conversion, end)) = parse_directive(format, after_percent, search) else {
        literal.push('%');
        return after_percent;
    };

    push_literal(segments, literal);
    segments.push(Segment::Directive {
        field: field.to_owned(),
        conversion,
    });
    end
}

/// Parse the directive whose opening parenthesis is at `open`.
///
/// Returns the field name, the conversion, and the offset just past the
/// conversion character when the directive is supported.
fn parse_directive<'a>(
    format: &'a str,
    open: usize,
    search: &mut ClosingParenthesisSearch,
) -> Option<(&'a str, Conversion, usize)> {
    if !format[open..].starts_with('(') {
        return None;
    }

    let close = search.find(format, open + 1)?;
    let conversion_start = close + 1;
    let conversion_char = format[conversion_start..].chars().next()?;
    let conversion = Conversion::from_char(conversion_char)?;
    Some((
        &format[open + 1..close],
        conversion,
        conversion_start + conversion_char.len_utf8(),
    ))
}

/// Move buffered literal text into `segments` when it is non-empty.
fn push_literal(segments: &mut Vec<Segment>, literal: &mut String) {
    if !literal.is_empty() {
        segments.push(Segment::Literal(std::mem::take(literal)));
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

        let close = format[from..].find(')').map(|relative| from + relative);
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
