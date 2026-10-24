use super::{NativeError, NativeErrorKind};
use crate::{Document, Schemas, Value as DamlValue};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Fixed manifest filename, outside the `.daml` definition namespace.
pub const BUNDLE_MANIFEST: &str = "daml.bundle";
pub(crate) const MAX_SOURCE_TABLE_ENTRIES: usize = 10_000;
pub(crate) const MAX_SOURCE_TABLE_BYTES: usize = crate::schema::profile::MAX_SOURCE_BYTES;
const MAX_LOGICAL_PATH_BYTES: usize = 4096;

#[derive(Debug)]
enum BundleResources {
    Filesystem,
    Supplied(Arc<HashMap<PathBuf, Arc<str>>>),
}

/// An explicitly selected native bundle root and its local import bindings.
#[derive(Debug)]
pub struct Bundle {
    pub(crate) root: PathBuf,
    pub(crate) imports: HashMap<String, PathBuf>,
    pub(crate) source_bytes: usize,
    resources: BundleResources,
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
        let imports = parse_manifest(&source, &manifest, |locator| {
            let path = Path::new(locator);
            Ok(if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            })
        })?;
        Ok(Self {
            root,
            imports,
            source_bytes,
            resources: BundleResources::Filesystem,
        })
    }

    /// Admits a bounded in-memory resource table rooted at `.` or a slash-relative directory.
    pub fn from_sources(
        root: impl AsRef<Path>,
        sources: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, NativeError> {
        let root_text = root.as_ref().to_str().ok_or_else(|| {
            NativeError::new(
                NativeErrorKind::InvalidManifest,
                "logical bundle root is not UTF-8",
            )
        })?;
        let root = logical_path(root_text, true)?;
        let mut table = HashMap::new();
        let mut source_bytes = 0usize;
        let mut count = 0usize;
        for (key, source) in sources {
            count = count
                .checked_add(1)
                .ok_or_else(|| source_table_limit("resource count"))?;
            if count > MAX_SOURCE_TABLE_ENTRIES {
                return Err(source_table_limit("resource count"));
            }
            let path = logical_path(&key, false)?;
            if path == root {
                return Err(NativeError::new(
                    NativeErrorKind::InvalidManifest,
                    format!(
                        "resource path {key:?} does not name a resource below logical namespace"
                    ),
                ));
            }
            source_bytes = source_bytes
                .checked_add(source.len())
                .ok_or_else(|| source_table_limit("source bytes"))?;
            if source_bytes > MAX_SOURCE_TABLE_BYTES {
                return Err(source_table_limit("source bytes"));
            }
            if table
                .insert(path.clone(), Arc::<str>::from(source))
                .is_some()
            {
                return Err(NativeError::new(
                    NativeErrorKind::InvalidManifest,
                    format!("duplicate logical resource path {}", path.display()),
                )
                .at_path(path));
            }
        }
        let resources = Arc::new(table);
        let manifest = root.join(BUNDLE_MANIFEST);
        let source = resources.get(&manifest).ok_or_else(|| {
            NativeError::new(
                NativeErrorKind::InvalidManifest,
                "supplied bundle has no root daml.bundle",
            )
            .at_path(&manifest)
        })?;
        let source = source.as_ref();
        let imports = parse_manifest(source, &manifest, |locator| logical_locator(&root, locator))?;
        let source_bytes = source.len();
        Ok(Self {
            root,
            imports,
            source_bytes,
            resources: BundleResources::Supplied(resources),
        })
    }

    pub(crate) fn canonical_root(&self, root: &Path) -> Result<PathBuf, NativeError> {
        match &self.resources {
            BundleResources::Filesystem => {
                fs::canonicalize(root).map_err(|error| NativeError::io(root, error))
            }
            BundleResources::Supplied(_) => {
                if root.starts_with("/") {
                    Ok(root.to_path_buf())
                } else {
                    Err(NativeError::new(
                        NativeErrorKind::PathEscape,
                        "logical resource resolves outside supplied namespace",
                    )
                    .at_path(root))
                }
            }
        }
    }

    pub(crate) fn canonical_file(&self, file: &Path) -> Result<PathBuf, NativeError> {
        match &self.resources {
            BundleResources::Filesystem => fs::canonicalize(file).map_err(|error| {
                NativeError::new(
                    NativeErrorKind::MissingDefinition,
                    format!("cannot load definition: {error}"),
                )
                .at_path(file)
                .caused_by(error)
            }),
            BundleResources::Supplied(resources) => {
                if resources.contains_key(file) {
                    Ok(file.to_path_buf())
                } else {
                    Err(NativeError::new(
                        NativeErrorKind::MissingDefinition,
                        "supplied definition resource is missing",
                    )
                    .at_path(file))
                }
            }
        }
    }

    pub(crate) fn has_manifest(&self, path: &Path) -> bool {
        match &self.resources {
            BundleResources::Filesystem => path.join(BUNDLE_MANIFEST).is_file(),
            BundleResources::Supplied(resources) => {
                resources.contains_key(&path.join(BUNDLE_MANIFEST))
            }
        }
    }

    pub(crate) fn read_resource(&self, path: &Path, limit: usize) -> Result<String, NativeError> {
        match &self.resources {
            BundleResources::Filesystem => read_source(path, limit),
            BundleResources::Supplied(resources) => {
                let source = resources.get(path).ok_or_else(|| {
                    NativeError::new(
                        NativeErrorKind::MissingDefinition,
                        "supplied source resource is missing",
                    )
                    .at_path(path)
                })?;
                if source.len() > limit {
                    return Err(source_limit(path));
                }
                Ok(source.to_string())
            }
        }
    }

    pub(crate) fn load_imports(
        &self,
        root: &Path,
    ) -> Result<(PathBuf, HashMap<String, PathBuf>, usize), NativeError> {
        match &self.resources {
            BundleResources::Filesystem => {
                let selected = Self::open(root)?;
                Ok((selected.root, selected.imports, selected.source_bytes))
            }
            BundleResources::Supplied(resources) => {
                let root = self.canonical_root(root)?;
                let manifest = root.join(BUNDLE_MANIFEST);
                let source = resources.get(&manifest).ok_or_else(|| {
                    NativeError::new(
                        NativeErrorKind::InvalidManifest,
                        "imported bundle has no daml.bundle",
                    )
                    .at_path(&manifest)
                })?;
                let imports =
                    parse_manifest(source, &manifest, |locator| logical_locator(&root, locator))?;
                Ok((root, imports, source.len()))
            }
        }
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

fn logical_path(value: &str, root: bool) -> Result<PathBuf, NativeError> {
    let invalid = || {
        NativeError::new(
            NativeErrorKind::InvalidManifest,
            format!("invalid logical resource path {value:?}"),
        )
    };
    if value.len() > MAX_LOGICAL_PATH_BYTES {
        return Err(source_table_limit("path bytes"));
    }
    if value.contains('\\') || value.contains('\0') || value.starts_with('/') || value.contains(':')
    {
        return Err(invalid());
    }
    if root && value == "." {
        return Ok(PathBuf::from("/"));
    }
    if value.is_empty() {
        return Err(invalid());
    }
    let mut out = PathBuf::from("/");
    for segment in value.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(invalid());
        }
        out.push(segment);
    }
    Ok(out)
}

fn parse_manifest(
    source: &str,
    manifest: &Path,
    mut resolve: impl FnMut(&str) -> Result<PathBuf, NativeError>,
) -> Result<HashMap<String, PathBuf>, NativeError> {
    let document = crate::parse(source).map_err(|error| {
        NativeError::new(
            NativeErrorKind::InvalidManifest,
            format!("invalid bundle manifest: {error}"),
        )
        .at_path(manifest)
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
            .at_path(manifest)
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
            .at_path(manifest)
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
            .at_path(manifest)
            .at_span(entry.value.span)
        })?;
        if locator.contains("://") || locator.starts_with("pkg:") || locator.contains('\0') {
            return Err(NativeError::new(
                NativeErrorKind::InvalidManifest,
                format!("bundle locator must be a local path, got {locator:?}"),
            )
            .at_path(manifest)
            .at_span(entry.value.span));
        }
        imports.insert(alias.to_owned(), resolve(locator)?);
    }
    Ok(imports)
}
fn logical_locator(root: &Path, locator: &str) -> Result<PathBuf, NativeError> {
    if locator.len() > MAX_LOGICAL_PATH_BYTES {
        return Err(source_table_limit("path bytes"));
    }
    if locator.starts_with('/')
        || locator.contains('\\')
        || locator.contains('\0')
        || locator.contains(':')
    {
        return Err(NativeError::new(
            NativeErrorKind::InvalidManifest,
            format!("invalid logical import path {locator:?}"),
        ));
    }
    let mut result = root.to_path_buf();
    for segment in locator.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if result == Path::new("/") || !result.pop() {
                    return Err(NativeError::new(
                        NativeErrorKind::PathEscape,
                        "logical import path escapes supplied resource namespace",
                    ));
                }
            }
            part => result.push(part),
        }
    }
    if result.starts_with("/") {
        Ok(result)
    } else {
        Err(NativeError::new(
            NativeErrorKind::PathEscape,
            "logical import path escapes supplied resource namespace",
        ))
    }
}

fn source_table_limit(subject: &str) -> NativeError {
    let bound = match subject {
        "resource count" => MAX_SOURCE_TABLE_ENTRIES,
        "source bytes" => MAX_SOURCE_TABLE_BYTES,
        "path bytes" => MAX_LOGICAL_PATH_BYTES,
        _ => 0,
    };
    NativeError::new(
        NativeErrorKind::LimitExceeded,
        format!("supplied resource table exceeds {subject} bound ({bound})"),
    )
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
