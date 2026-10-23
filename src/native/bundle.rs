use super::{NativeError, NativeErrorKind};
use crate::{Document, Schemas, Value as DamlValue};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// Fixed manifest filename, outside the `.daml` definition namespace.
pub const BUNDLE_MANIFEST: &str = "daml.bundle";

/// An explicitly selected native bundle root and its local import bindings.
#[derive(Debug)]
pub struct Bundle {
    pub(crate) root: PathBuf,
    pub(crate) imports: HashMap<String, PathBuf>,
    pub(crate) source_bytes: usize,
}

impl Bundle {
    /// Opens `root` and admits its required DAML mapping in `daml.bundle`.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, NativeError> {
        let supplied = root.as_ref();
        let root = fs::canonicalize(supplied).map_err(|error| NativeError::io(supplied, error))?;
        if !root.is_dir() {
            return Err(NativeError::new(
                NativeErrorKind::InvalidManifest,
                "bundle root is not a directory",
            )
            .at_path(root));
        }
        let manifest = root.join(BUNDLE_MANIFEST);
        let source = read_source(&manifest, crate::schema::profile::MAX_SOURCE_BYTES)?;
        let source_bytes = source.len();
        let document = crate::parse(source).map_err(|error| {
            NativeError::new(
                NativeErrorKind::InvalidManifest,
                format!("invalid bundle manifest: {error}"),
            )
            .at_path(&manifest)
            .at_span(error.span)
            .caused_by(error)
        })?;
        let entries = match (&document.root().tag, &document.root().value) {
            (None, DamlValue::Mapping(entries)) => entries,
            _ => {
                return Err(NativeError::new(
                    NativeErrorKind::InvalidManifest,
                    "bundle manifest must be a flat mapping",
                )
                .at_path(&manifest)
                .at_span(document.root().span));
            }
        };
        let mut imports = HashMap::with_capacity(entries.len());
        for entry in entries {
            let alias = document.slice(entry.key).unwrap_or("");
            if alias == "self" || !crate::names::valid_segment(alias) {
                return Err(NativeError::new(
                    NativeErrorKind::InvalidName,
                    format!("invalid or reserved bundle alias {alias:?}"),
                )
                .at_path(&manifest)
                .at_span(entry.key));
            }
            let locator = if entry.value.tag.is_none() {
                document.string(&entry.value)
            } else {
                None
            }
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                NativeError::new(
                    NativeErrorKind::InvalidManifest,
                    format!("bundle locator for {alias:?} must be an untagged nonempty string"),
                )
                .at_path(&manifest)
                .at_span(entry.value.span)
            })?;
            let path = Path::new(locator);
            if locator.contains("://") || locator.starts_with("pkg:") || locator.contains('\0') {
                return Err(NativeError::new(
                    NativeErrorKind::InvalidManifest,
                    format!("bundle locator must be a local path, got {locator:?}"),
                )
                .at_path(&manifest)
                .at_span(entry.value.span));
            }
            let anchored = if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            };
            imports.insert(alias.to_owned(), anchored);
        }
        Ok(Self {
            root,
            imports,
            source_bytes,
        })
    }
    /// Discovers the nearest ancestor manifest from a document's directory.
    ///
    /// The document may be unsaved; its containing directory must exist.
    pub fn discover(document_path: impl AsRef<Path>) -> Result<Option<Self>, NativeError> {
        let supplied = document_path.as_ref();
        let directory = if supplied.is_dir() {
            supplied
        } else {
            supplied
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
        };
        let mut directory =
            fs::canonicalize(directory).map_err(|error| NativeError::io(directory, error))?;
        loop {
            let manifest = directory.join(BUNDLE_MANIFEST);
            if manifest.is_file() {
                return Self::open(&directory).map(Some);
            }
            if !directory.pop() {
                return Ok(None);
            }
        }
    }

    /// Compiles the reachable native definitions for a configuration.
    pub fn compile(
        &self,
        document: &Document,
        expected: Option<&str>,
    ) -> Result<Schemas, NativeError> {
        super::compiler::compile(self, document, expected)
    }
}

pub(super) fn read_source(path: &Path, limit: usize) -> Result<String, NativeError> {
    let file = fs::File::open(path).map_err(|error| NativeError::io(path, error))?;
    let length = file
        .metadata()
        .map_err(|error| NativeError::io(path, error))?
        .len();
    if length > limit as u64 {
        return Err(source_limit(path));
    }
    let mut source = String::with_capacity(length as usize);
    file.take(limit as u64 + 1)
        .read_to_string(&mut source)
        .map_err(|error| NativeError::io(path, error))?;
    if source.len() > limit {
        return Err(source_limit(path));
    }
    Ok(source)
}

fn source_limit(path: &Path) -> NativeError {
    NativeError::new(
        NativeErrorKind::LimitExceeded,
        "native source exceeds the remaining byte budget",
    )
    .at_path(path)
}
