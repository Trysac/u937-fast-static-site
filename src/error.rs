use std::fmt;

/// Machine-readable error code for [`SiteError`].
pub type ErrorCode = &'static str;

/// Error returned by site generation, template parsing/rendering, or front-matter parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteError {
    /// Stable machine-readable error code (`template_syntax`, `missing_template`,
    /// `include_cycle`, `unknown_variable`, `front_matter`, `input_too_large`, `io`).
    pub code: ErrorCode,
    /// Relative path associated with the error, if known.
    pub path: Option<String>,
    /// 1-based line number associated with the error, if known.
    pub line: Option<usize>,
    /// Short human-readable diagnostic message (truncated to at most 200 chars).
    pub message: String,
}

impl SiteError {
    /// Creates a new [`SiteError`] with a code and message.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        let mut msg = message.into();
        if msg.len() > 200 {
            msg.truncate(200);
        }
        Self {
            code,
            path: None,
            line: None,
            message: msg,
        }
    }

    /// Attaches a file path to this error.
    #[must_use]
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Attaches a 1-based line number to this error.
    #[must_use]
    pub fn with_line(mut self, line: usize) -> Self {
        self.line = Some(line);
        self
    }
}

impl fmt::Display for SiteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}]", self.code)?;
        if let Some(ref p) = self.path {
            write!(f, " {p}")?;
            if let Some(l) = self.line {
                write!(f, ":{l}")?;
            }
        } else if let Some(l) = self.line {
            write!(f, " line {l}")?;
        }
        write!(f, ": {}", self.message)
    }
}

impl std::error::Error for SiteError {}
