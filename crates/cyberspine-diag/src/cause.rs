use alloc::string::String;
use core::fmt;

use crate::{Code, Diagnostic};

/// A failure reported across a plugin boundary.
///
/// A body or an arbiter knows *what* broke, never *where* in the run: it has no
/// tick, no instance, no image path. So it reports a cause, and the host — the
/// only layer that knows the session — turns it into a [`Diagnostic`] by
/// attaching an anchor. The type keeps that division honest: there is no place
/// field here to fill in wrongly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cause {
    pub code: Code,
    pub message: String,
    pub help: Option<String>,
}

impl Cause {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            help: None,
        }
    }

    pub fn with_help(mut self, text: impl Into<String>) -> Self {
        self.help = Some(text.into());
        self
    }
}

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some(help) = &self.help {
            write!(f, " (help: {help})")?;
        }
        Ok(())
    }
}

impl From<Cause> for Diagnostic {
    fn from(cause: Cause) -> Self {
        let mut diagnostic = Diagnostic::error(cause.code, cause.message);
        diagnostic.help = cause.help;
        diagnostic
    }
}
