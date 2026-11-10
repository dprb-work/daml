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
    #[cfg(feature = "lsp")]
    Editor {
        overlays: Arc<HashMap<PathBuf, Option<String>>>,
        manifest: Option<String>,
    },
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
            #[cfg(feature = "lsp")]
            BundleResources::Editor { .. } => {
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
            #[cfg(feature = "lsp")]
            BundleResources::Editor { overlays, .. } => {
                let canonical = editor_path(file)?;
                if overlays.contains_key(&canonical) || canonical.is_file() {
                    Ok(canonical)
                } else {
                    Err(NativeError::new(
                        NativeErrorKind::MissingDefinition,
                        "definition source is missing",
                    )
                    .at_path(file))
                }
            }
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
            #[cfg(feature = "lsp")]
            BundleResources::Editor { overlays, .. } => {
                let manifest = path.join(BUNDLE_MANIFEST);
                editor_path(&manifest).is_ok_and(|canonical| overlays.contains_key(&canonical))
                    || manifest.is_file()
            }
            BundleResources::Supplied(resources) => {
                resources.contains_key(&path.join(BUNDLE_MANIFEST))
            }
        }
    }

    pub(crate) fn read_resource(&self, path: &Path, limit: usize) -> Result<String, NativeError> {
        match &self.resources {
            BundleResources::Filesystem => read_source(path, limit),
            #[cfg(feature = "lsp")]
            BundleResources::Editor {
                overlays, manifest, ..
            } => {
                if path.parent() == Some(self.root.as_path())
                    && path.file_name().is_some_and(|name| name == BUNDLE_MANIFEST)
                    && let Some(manifest) = manifest
                {
                    if manifest.len() > limit {
                        return Err(source_limit(path));
                    }
                    Ok(manifest.clone())
                } else {
                    read_editor_source(overlays, path, limit).map(std::borrow::Cow::into_owned)
                }
            }
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
            #[cfg(feature = "lsp")]
            BundleResources::Editor { .. } => {
                let root = self.canonical_root(root)?;
                let manifest = root.join(BUNDLE_MANIFEST);
                let source = self.read_resource(&manifest, MAX_SOURCE_TABLE_BYTES)?;
                let imports = self.imports_from_source(&root, &source)?;
                Ok((root, imports, source.len()))
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
        self.compile_with_types(document, expected, std::iter::empty())
    }

    /// Compiles reachable definitions plus explicitly selected entry-visible types.
    ///
    /// Additional types are loaded but do not impose a root resolution policy.
    /// Their caller-supplied names have no configuration source span.
    pub fn compile_with_types<'names>(
        &self,
        document: &Document,
        expected: Option<&str>,
        additional_types: impl IntoIterator<Item = &'names str>,
    ) -> Result<Schemas, NativeError> {
        super::compiler::compile(self, document, expected, additional_types)
    }

    #[cfg(feature = "lsp")]
    pub(crate) fn open_editor(
        root: &Path,
        overlays: Vec<(PathBuf, Option<&str>)>,
    ) -> Result<Self, NativeError> {
        Self::open_editor_admitted(root, admit_overlays(overlays)?)
    }

    #[cfg(feature = "lsp")]
    fn open_editor_admitted(
        root: &Path,
        overlays: Arc<HashMap<PathBuf, Option<String>>>,
    ) -> Result<Self, NativeError> {
        let root = fs::canonicalize(root).map_err(|error| NativeError::io(root, error))?;
        if !root.is_dir() {
            return Err(NativeError::new(
                NativeErrorKind::InvalidManifest,
                "bundle root is not a directory",
            )
            .at_path(root));
        }
        let manifest_path = root.join(BUNDLE_MANIFEST);
        let source = read_editor_source(&overlays, &manifest_path, MAX_SOURCE_TABLE_BYTES)?;
        let source_bytes = source.len();
        let imports = parse_manifest(&source, &manifest_path, |locator| {
            let path = Path::new(locator);
            Ok(if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            })
        })?;
        // Disk text is captured once; accepted overlay text is already owned by
        // the immutable table and must not receive a duplicate root snapshot.
        let manifest = match source {
            std::borrow::Cow::Borrowed(_) => None,
            std::borrow::Cow::Owned(source) => Some(source),
        };
        Ok(Self {
            root,
            imports,
            source_bytes,
            resources: BundleResources::Editor { overlays, manifest },
        })
    }

    #[cfg(feature = "lsp")]
    pub(crate) fn discover_editor(
        document_path: &Path,
        overlays: Vec<(PathBuf, Option<&str>)>,
        attempted: &mut Vec<PathBuf>,
    ) -> Result<Option<Self>, NativeError> {
        let overlays = admit_overlays(overlays)?;
        let path = editor_path(document_path)?;
        let mut directory = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(Path::new("/")).to_path_buf()
        };
        let mut budget = super::tooling::EditorBudget::default();
        loop {
            let manifest = directory.join(BUNDLE_MANIFEST);
            budget.charge(1, manifest.as_os_str().len(), &manifest)?;
            let selected = overlays.contains_key(&editor_path(&manifest)?) || manifest.is_file();
            attempted.push(manifest);
            if selected {
                return Self::open_editor_admitted(&directory, overlays).map(Some);
            }
            if !directory.pop() {
                return Ok(None);
            }
        }
    }

    #[cfg(feature = "lsp")]
    pub(super) fn imports_from_source(
        &self,
        root: &Path,
        source: &str,
    ) -> Result<HashMap<String, PathBuf>, NativeError> {
        parse_manifest(source, &root.join(BUNDLE_MANIFEST), |locator| {
            if matches!(&self.resources, BundleResources::Supplied(_)) {
                logical_locator(root, locator)
            } else {
                let path = Path::new(locator);
                Ok(if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    root.join(path)
                })
            }
        })
    }

    #[cfg(feature = "lsp")]
    pub(super) fn editor_source_path(&self, path: &Path) -> Result<PathBuf, NativeError> {
        match &self.resources {
            BundleResources::Supplied(_) => Ok(path.to_path_buf()),
            _ => editor_path(path),
        }
    }

    #[cfg(feature = "lsp")]
    pub(crate) fn editor_definition_name(&self, path: &Path) -> Result<String, NativeError> {
        if path.starts_with(&self.root) {
            self.editor_check_route(&self.root, path, false)?;
        }
        let path = match &self.resources {
            BundleResources::Supplied(_) => self.canonical_file(path)?,
            _ => editor_path(path)?,
        };
        self.editor_check_route(&self.root, &path, false)?;
        let relative = path.strip_prefix(&self.root).map_err(|_| {
            NativeError::new(
                NativeErrorKind::PathEscape,
                "definition is outside selected bundle",
            )
            .at_path(&path)
        })?;
        if relative
            .extension()
            .is_none_or(|extension| extension != "daml")
        {
            return Err(NativeError::new(
                NativeErrorKind::InvalidName,
                "native definition must have the .daml extension",
            )
            .at_path(path));
        }
        let mut name = String::from("self");
        let stem = relative.with_extension("");
        for component in stem.components() {
            let segment = component
                .as_os_str()
                .to_str()
                .filter(|name| crate::names::valid_segment(name))
                .ok_or_else(|| {
                    NativeError::new(
                        NativeErrorKind::InvalidName,
                        "invalid native definition path",
                    )
                    .at_path(&path)
                })?;
            if name.len().saturating_add(segment.len() + 1) > MAX_LOGICAL_PATH_BYTES {
                return Err(source_table_limit("path bytes").at_path(&path));
            }
            name.push('.');
            name.push_str(segment);
        }
        if name == "self" {
            return Err(
                NativeError::new(NativeErrorKind::InvalidName, "empty definition name")
                    .at_path(path),
            );
        }
        Ok(name)
    }

    #[cfg(feature = "lsp")]
    fn editor_check_route(
        &self,
        root: &Path,
        path: &Path,
        directory: bool,
    ) -> Result<(), NativeError> {
        if !path.starts_with(root) {
            return Err(NativeError::new(
                NativeErrorKind::PathEscape,
                "definition route escapes selected bundle",
            )
            .at_path(path));
        }
        let mut parent = if directory { Some(path) } else { path.parent() };
        while let Some(current) = parent {
            if current == root {
                return Ok(());
            }
            if self.has_manifest(current) {
                return Err(NativeError::new(
                    NativeErrorKind::NestedBundleBoundary,
                    "definition route crosses a nested bundle; use an explicit import",
                )
                .at_path(current.join(BUNDLE_MANIFEST)));
            }
            parent = current.parent();
        }
        Err(NativeError::new(
            NativeErrorKind::PathEscape,
            "definition route escapes selected bundle",
        )
        .at_path(path))
    }

    #[cfg(feature = "lsp")]
    pub(crate) fn editor_visible_names(&self, prefix: &str) -> Result<Vec<String>, NativeError> {
        use super::tooling::EditorBudget;
        use std::collections::BTreeSet;
        let mut budget = EditorBudget::default();
        if prefix.len() > MAX_LOGICAL_PATH_BYTES {
            return Err(source_table_limit("path bytes"));
        }
        let mut names = BTreeSet::new();
        for name in super::compiler::EDITOR_BUILTINS {
            if name.starts_with(prefix) {
                budget.charge(1, name.len(), &self.root)?;
                names.insert((*name).to_owned());
            }
        }
        for alias in std::iter::once("self").chain(self.imports.keys().map(String::as_str)) {
            if alias.starts_with(prefix) || prefix == alias {
                budget.charge(1, alias.len() + 1, &self.root)?;
                names.insert(format!("{alias}."));
            }
        }
        let Some((namespace, partial)) = prefix.rsplit_once('.') else {
            return Ok(names.into_iter().collect());
        };
        let mut segments = namespace.split('.');
        let alias = segments.next().unwrap_or("");
        let selected = if alias == "self" {
            &self.root
        } else {
            self.imports.get(alias).ok_or_else(|| {
                NativeError::new(NativeErrorKind::UnknownReference, "unknown bundle alias")
                    .for_reference(prefix)
            })?
        };
        let root = self.canonical_root(selected)?;
        if root != self.root {
            self.load_imports(&root)?;
        }
        let mut directory = root.clone();
        for segment in segments {
            if !crate::names::valid_segment(segment) {
                return Err(NativeError::new(
                    NativeErrorKind::InvalidName,
                    "invalid definition prefix",
                )
                .for_reference(prefix));
            }
            directory.push(segment);
        }
        self.editor_check_route(&root, &directory, true)?;
        let canonical = match &self.resources {
            BundleResources::Supplied(_) => directory.clone(),
            _ => editor_path(&directory)?,
        };
        self.editor_check_route(&root, &canonical, true)?;
        let mut children = BTreeSet::new();
        let mut add = |path: &Path| -> Result<(), NativeError> {
            let Ok(relative) = path.strip_prefix(&canonical) else {
                return Ok(());
            };
            let Some(component) = relative.components().next() else {
                return Ok(());
            };
            let child = component.as_os_str();
            budget.charge(1, child.len(), &canonical)?;
            children.insert(child.to_os_string());
            Ok(())
        };
        match &self.resources {
            BundleResources::Supplied(resources) => {
                for path in resources.keys() {
                    add(path)?;
                }
            }
            _ => {
                match fs::read_dir(&canonical) {
                    Ok(entries) => {
                        for entry in entries {
                            let entry =
                                entry.map_err(|error| NativeError::io(&canonical, error))?;
                            add(&entry.path())?;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(NativeError::io(&canonical, error)),
                }
                if let BundleResources::Editor { overlays, .. } = &self.resources {
                    for path in overlays.keys() {
                        add(path)?;
                    }
                }
            }
        }
        for child in children {
            let path = canonical.join(&child);
            let Some(text) = child.to_str() else {
                continue;
            };
            let file = text.strip_suffix(".daml");
            let segment = file.unwrap_or(text);
            if !crate::names::valid_segment(segment) || !segment.starts_with(partial) {
                continue;
            }
            let is_directory = file.is_none();
            let actual = match &self.resources {
                BundleResources::Supplied(_) => path.clone(),
                _ => editor_path(&path)?,
            };
            if self.editor_check_route(&root, &path, is_directory).is_err()
                || self
                    .editor_check_route(&root, &actual, is_directory)
                    .is_err()
            {
                continue;
            }
            if is_directory {
                let visible_directory = match &self.resources {
                    BundleResources::Supplied(resources) => {
                        resources.keys().any(|entry| entry.starts_with(&path))
                    }
                    BundleResources::Editor { overlays, .. } => {
                        path.is_dir() || overlays.keys().any(|entry| entry.starts_with(&path))
                    }
                    BundleResources::Filesystem => path.is_dir(),
                };
                if !visible_directory {
                    continue;
                }
            }
            budget.charge(
                1,
                namespace.len() + segment.len() + 1 + usize::from(is_directory),
                &canonical,
            )?;
            names.insert(format!(
                "{namespace}.{segment}{}",
                if is_directory { "." } else { "" }
            ));
        }
        Ok(names.into_iter().collect())
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

// Resolve real components first so a symlink followed by `..` keeps filesystem
// semantics. Missing suffixes are allowed for unsaved buffers, not disk fallback.
#[cfg(feature = "lsp")]
pub(super) fn editor_path(path: &Path) -> Result<PathBuf, NativeError> {
    use std::path::Component;
    if !path.is_absolute() || path.as_os_str().len() > MAX_LOGICAL_PATH_BYTES {
        return Err(NativeError::new(
            NativeErrorKind::PathEscape,
            "editor source path must be a bounded absolute local path",
        )
        .at_path(path));
    }
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => resolved.push(prefix.as_os_str()),
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(name) => {
                resolved.push(name);
                match fs::symlink_metadata(&resolved) {
                    Ok(_) => {
                        resolved = fs::canonicalize(&resolved)
                            .map_err(|error| NativeError::io(&resolved, error))?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(NativeError::io(&resolved, error)),
                }
            }
        }
    }
    if resolved.as_os_str().len() > MAX_LOGICAL_PATH_BYTES {
        return Err(source_table_limit("path bytes").at_path(path));
    }
    Ok(resolved)
}

// Preserve an entry's parent filesystem identity without following its leaf.
#[cfg(feature = "lsp")]
fn editor_watch_path(path: &Path) -> Result<PathBuf, NativeError> {
    let Some((parent, name)) = path.parent().zip(path.file_name()) else {
        return editor_path(path);
    };
    if path.as_os_str().len() > MAX_LOGICAL_PATH_BYTES {
        return Err(source_table_limit("path bytes").at_path(path));
    }
    let mut resolved = editor_path(parent)?;
    let separator = usize::from(
        !resolved
            .as_os_str()
            .as_encoded_bytes()
            .last()
            .is_some_and(|byte| std::path::is_separator(char::from(*byte))),
    );
    if resolved
        .as_os_str()
        .len()
        .saturating_add(separator)
        .saturating_add(name.len())
        > MAX_LOGICAL_PATH_BYTES
    {
        return Err(source_table_limit("path bytes").at_path(path));
    }
    resolved.push(name);
    Ok(resolved)
}

/// Stream file-link entries and their terminal dependency, including missing
/// targets. Stop when an entry was already visited; this also closes cycles.
#[cfg(feature = "lsp")]
pub(crate) fn editor_watch_paths(
    path: &Path,
) -> impl Iterator<Item = Result<PathBuf, NativeError>> + '_ {
    let mut next: Option<std::borrow::Cow<'_, Path>> = Some(std::borrow::Cow::Borrowed(path));
    let mut remaining = MAX_SOURCE_TABLE_ENTRIES;
    std::iter::from_fn(move || {
        let path = next.take()?;
        if remaining == 0 {
            return Some(Err(
                source_table_limit("watch path count").at_path(path.as_ref())
            ));
        }
        remaining -= 1;
        Some((|| {
            let entry = editor_watch_path(&path)?;
            match fs::symlink_metadata(&entry) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    let target =
                        fs::read_link(&entry).map_err(|error| NativeError::io(&entry, error))?;
                    if target.as_os_str().len() > MAX_LOGICAL_PATH_BYTES {
                        return Err(source_table_limit("path bytes").at_path(&entry));
                    }
                    let target = if target.is_absolute() {
                        target
                    } else {
                        entry
                            .parent()
                            .expect("absolute link entry has a parent")
                            .join(target)
                    };
                    if target.as_os_str().len() > MAX_LOGICAL_PATH_BYTES {
                        return Err(source_table_limit("path bytes").at_path(&entry));
                    }
                    next = Some(std::borrow::Cow::Owned(target));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(NativeError::io(&entry, error)),
            }
            Ok(entry)
        })())
    })
}

#[cfg(feature = "lsp")]
fn admit_overlays(
    overlays: Vec<(PathBuf, Option<&str>)>,
) -> Result<Arc<HashMap<PathBuf, Option<String>>>, NativeError> {
    let mut budget = super::tooling::EditorBudget::default();
    let mut admitted: HashMap<PathBuf, Option<String>> = HashMap::new();
    for (path, source) in overlays {
        budget.charge(
            1,
            path.as_os_str()
                .len()
                .saturating_add(source.map_or(0, str::len)),
            &path,
        )?;
        let canonical = editor_path(&path)?;
        budget.charge(0, canonical.as_os_str().len(), &canonical)?;
        if canonical.is_dir() {
            return Err(NativeError::new(
                NativeErrorKind::InvalidDefinition,
                "editor source identifies an existing directory, not a file",
            )
            .at_path(canonical));
        }
        if let Some(existing) = admitted.get(&canonical) {
            if existing.as_deref() == source {
                continue;
            }
            return Err(NativeError::new(
                NativeErrorKind::InvalidDefinition,
                "conflicting editor overlays resolve to the same source",
            )
            .at_path(canonical));
        }
        admitted.insert(canonical, source.map(str::to_owned));
    }
    Ok(Arc::new(admitted))
}

#[cfg(feature = "lsp")]
fn read_editor_source<'a>(
    overlays: &'a HashMap<PathBuf, Option<String>>,
    path: &Path,
    limit: usize,
) -> Result<std::borrow::Cow<'a, str>, NativeError> {
    let canonical = editor_path(path)?;
    match overlays.get(&canonical) {
        Some(Some(source)) => {
            if source.len() > limit {
                Err(source_limit(&canonical))
            } else {
                Ok(std::borrow::Cow::Borrowed(source))
            }
        }
        Some(None) => Err(NativeError::new(
            NativeErrorKind::LimitExceeded,
            "current open editor source was rejected; disk fallback is forbidden",
        )
        .at_path(canonical)),
        None => read_source(path, limit).map(std::borrow::Cow::Owned),
    }
}
