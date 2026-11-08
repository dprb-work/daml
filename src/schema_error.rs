use crate::SchemaLocation;
use std::{error::Error, fmt};

/// A schema loading/composition failure category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaErrorKind {
    /// A schema document was not valid JSON or DAML.
    Parse,
    /// A module envelope or schema declaration is malformed.
    InvalidSchema,
    /// Two selected modules declare the same type or resource.
    DuplicateName,
    /// A referenced resource or type was not explicitly supplied.
    UnknownReference,
    /// A reference loop can evaluate repeatedly without descending into the instance.
    ReferenceCycle,
    /// Default placement would require undefined/ambiguous evaluation semantics.
    UnsupportedDefaults,
    /// A declared schema exceeds implementation resource or numeric bounds.
    LimitExceeded,
}

/// A schema document or composition error, not an error in supplied data.
#[derive(Debug)]
pub struct SchemaError {
    /// Structured failure category.
    pub kind: SchemaErrorKind,
    /// Schema location when one is available.
    pub location: Option<SchemaLocation>,
    /// Human-readable diagnostic.
    pub message: String,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl SchemaError {
    pub(crate) fn new(kind: SchemaErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            location: None,
            message: message.into(),
            source: None,
        }
    }

    pub(crate) fn at(mut self, location: SchemaLocation) -> Self {
        self.location = Some(location);
        self
    }

    pub(crate) fn caused_by(mut self, source: impl Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }
}

impl fmt::Display for SchemaError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.message)?;
        if let Some(location) = &self.location {
            write!(out, " at {}#{}", location.resource, location.pointer)?;
        }
        Ok(())
    }
}

impl Error for SchemaError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}
