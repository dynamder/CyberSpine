use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Source text by path, read at most once per path.
///
/// A diagnostic names its artifact; drawing a caret needs the text behind that
/// path. The assembler hands its text in (it already parsed it), so a file is only
/// read from disk when nobody supplied it — a fault whose source lives next to the
/// image, for instance.
#[derive(Debug, Default)]
pub struct SourceCache {
    entries: HashMap<PathBuf, Option<String>>,
}

impl SourceCache {
    pub fn remember(&mut self, path: impl Into<PathBuf>, text: impl Into<String>) {
        self.entries.insert(path.into(), Some(text.into()));
    }

    /// The text at `artifact`, or `None` when it is unreadable.
    ///
    /// Absent is normal, not an error: a stripped image has no source path, a
    /// moved image may point at a source that is not on this machine, and a
    /// diagnostic about a byte offset has no source at all. Rendering degrades;
    /// it never fails.
    pub fn get(&mut self, artifact: &str) -> Option<&str> {
        let path = Path::new(artifact);
        self.entries
            .entry(path.to_path_buf())
            .or_insert_with(|| std::fs::read_to_string(path).ok())
            .as_deref()
    }
}

/// Byte range to annotate for a one-based line and column.
///
/// Normally one character. A position at the end of a line (a missing token)
/// becomes a zero-length span so the reported column is still exact.
pub fn span_at(source: &str, line: u32, column: u32) -> Option<Range<usize>> {
    let start = byte_offset(source, line, column)?;

    if start >= source.len() || matches!(source[start..].chars().next(), Some('\n' | '\r')) {
        return Some(start..start);
    }

    let end = start + source[start..].chars().next()?.len_utf8();
    Some(start..end)
}

fn byte_offset(source: &str, line: u32, column: u32) -> Option<usize> {
    let line = usize::try_from(line).ok()?;
    let column = usize::try_from(column).ok()?;
    let mut offset = 0usize;
    for (index, text) in source.split_inclusive('\n').enumerate() {
        if index + 1 == line {
            let chars = column.saturating_sub(1);
            let byte = text
                .char_indices()
                .nth(chars)
                .map_or(text.len(), |(index, _)| index);
            return Some(offset + byte);
        }
        offset += text.len();
    }
    None
}
