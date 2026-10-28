use crate::Span;
use std::{error::Error, fmt, io, path::PathBuf};

/// A native bundle loading or compilation failure category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeErrorKind {
    /// Filesystem operation failed.
    Io,
    /// The bundle manifest is missing, malformed, or has an invalid shape.
    InvalidManifest,
    /// A bundle alias, tag path, or definition name is invalid.
    InvalidName,
    /// A referenced definition file or supplied resource does not exist or cannot be read.
    MissingDefinition,
    /// A definition symlink or supplied logical resource path escapes its selected boundary.
    PathEscape,
    /// A definition path crosses a nested bundle without an explicit binding.
    NestedBundleBoundary,
    /// A referenced bundle alias or definition cannot be resolved.
    UnknownReference,
    /// A reachable definition graph contains a forbidden reference cycle.
    ReferenceCycle,
    /// A native definition is malformed or uses unsupported metadata.
    InvalidDefinition,
    /// A declared or inherited default contradicts its constraints.
    InvalidDefault,
    /// A resource or source bound was exceeded.
    LimitExceeded,
    /// In-memory schema admission or compilation failed.
    Schema,
}

/// A structured failure while selecting, loading, or compiling a native bundle.
#[derive(Debug)]
pub struct NativeError {
    /// Machine-readable category for caller branching.
    pub kind: NativeErrorKind,
    /// Manifest, definition, or logical resource identifier associated with the failure, when known.
    pub path: Option<PathBuf>,
    /// Original-source byte span associated with the failure, when known.
    pub span: Option<Span>,
    /// Tag or reference being resolved, when applicable.
    pub reference: Option<String>,
    /// Human-readable diagnostic preserving the underlying cause separately.
    pub message: String,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl NativeError {
    pub(crate) fn new(kind: NativeErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            path: None,
            span: None,
            reference: None,
            message: message.into(),
            source: None,
        }
    }

    pub(crate) fn at_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub(crate) fn at_span(mut self, span: impl Into<Option<Span>>) -> Self {
        self.span = span.into();
        self
    }

    pub(crate) fn for_reference(mut self, reference: impl Into<String>) -> Self {
        self.reference = Some(reference.into());
        self
    }

    pub(crate) fn caused_by(mut self, source: impl Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    pub(crate) fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self::new(NativeErrorKind::Io, error.to_string())
            .at_path(path)
            .caused_by(error)
    }
}

impl fmt::Display for NativeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.message)?;
        if let Some(path) = &self.path {
            write!(out, " at {}", path.display())?;
        }
        if let Some(span) = self.span {
            write!(out, " (bytes {}..{})", span.start, span.end)?;
        }
        if let Some(reference) = &self.reference {
            write!(out, " while resolving {reference:?}")?;
        }
        Ok(())
    }
}

impl Error for NativeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}
