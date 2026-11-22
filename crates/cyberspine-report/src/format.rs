/// Which shape a [`crate::Renderer`] writes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Format {
    /// A terminal: source frames, carets, one block per diagnostic.
    #[default]
    Human,
    /// A pipe: one JSON object per diagnostic, one per line.
    Json,
}

impl Format {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Json => "json",
        }
    }
}
