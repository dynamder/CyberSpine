use cyberspine_arbiter::Notice;
use cyberspine_diag::{Anchor, Diagnostic, Level};

/// A notice becomes a warning.
///
/// The arbiter says *what* and *which artifact*; the host decides that it is a
/// warning, and the report layer decides how it is drawn. A notice is never an
/// error: the run it describes succeeded.
pub fn into_diagnostic(notice: Notice) -> Diagnostic {
    let Notice { cause, artifact } = notice;
    let mut diagnostic = Diagnostic::new(Level::Warning, cause.code, cause.message);
    diagnostic.help = cause.help;
    if let Some(artifact) = artifact {
        diagnostic = diagnostic.at(Anchor::whole(artifact));
    }
    diagnostic
}
