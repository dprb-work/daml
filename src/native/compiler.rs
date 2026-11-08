use super::{Bundle, NativeError, NativeErrorKind};
use crate::{Document, Node, SchemaError, SchemaModule, Schemas, Span, StringValue, Value};
use serde_json::{Map, Number, Value as Json};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

type SchemaObject = Map<String, Json>;

#[derive(Clone, Copy)]
struct SourceContext<'a> {
    source: &'a str,
    root: &'a Path,
    file: &'a Path,
}
#[derive(Clone)]
enum SchemaSource {
    Type(String),
    Resource(usize),
}

struct DefaultCheck {
    schema_type: String,
    value: Json,
    annotations: Vec<Json>,
    path: PathBuf,
    span: Span,
}
#[derive(Clone)]
struct RegexOrigin {
    path: PathBuf,
    span: Span,
}
#[derive(Clone)]
struct PresetFragment {
    value: Json,
    annotations: Vec<Json>,
}

#[derive(Clone, Copy)]
struct InheritedDefaults<'a> {
    schemas: &'a [Json],
    presets: &'a [PresetFragment],
    root_replaced: bool,
}
#[derive(Clone, Hash, PartialEq, Eq)]
struct SchemaPosition {
    keyword: String,
    key: Option<String>,
    index: Option<usize>,
}
#[derive(Clone, Hash, PartialEq, Eq)]
enum DefaultPathStep {
    Property(String),
    Item(usize),
}
type DefaultPath = Vec<DefaultPathStep>;

struct WalkBudget<'a> {
    work: usize,
    copied_nodes: usize,
    copied_bytes: usize,
    file: &'a Path,
    span: Span,
}
impl WalkBudget<'_> {
    fn charge(&mut self, depth: usize) -> Result<(), NativeError> {
        self.work += 1;
        if self.work > crate::schema::profile::MAX_COMPILE_NODES {
            return Err(NativeError::new(
                NativeErrorKind::LimitExceeded,
                "native effective-default graph traversal exceeds compiler-node bound",
            )
            .at_path(self.file)
            .at_span(self.span));
        }
        if depth > crate::schema::profile::MAX_SCHEMA_DEPTH {
            return Err(NativeError::new(
                NativeErrorKind::LimitExceeded,
                "native effective-default graph traversal exceeds schema-depth bound",
            )
            .at_path(self.file)
            .at_span(self.span));
        }
        Ok(())
    }

    fn charge_copy(&mut self, value: &Json) -> Result<(), NativeError> {
        admit_copy(
            &mut self.copied_nodes,
            &mut self.copied_bytes,
            value,
            self.file,
            Some(self.span),
        )
    }
    fn charge_map_copy(&mut self, value: &Map<String, Json>) -> Result<(), NativeError> {
        let (nodes, bytes) = json_map_cost(value);
        admit_copy_cost(
            &mut self.copied_nodes,
            &mut self.copied_bytes,
            nodes,
            bytes,
            self.file,
            Some(self.span),
        )
    }
    fn charge_preset_copy(&mut self, preset: &PresetFragment) -> Result<(), NativeError> {
        self.charge_copy(&preset.value)?;
        for annotation in &preset.annotations {
            self.charge_copy(annotation)?;
        }
        Ok(())
    }
}
struct Compiler<'a> {
    bundle: &'a Bundle,
    contexts: HashMap<PathBuf, HashMap<String, PathBuf>>,
    schema_sources: HashMap<String, SchemaSource>,
    active: HashSet<String>,
    source_bytes: usize,
    types: Map<String, Json>,
    resources: Vec<Json>,
    registrations: HashMap<String, String>,
    builtin_tags: HashSet<String>,
    checks: Vec<DefaultCheck>,
    regex_origins: HashMap<String, RegexOrigin>,
    next_regex_origin: u64,
    default_projections: HashMap<(String, DefaultPath), String>,
    next_default_projection: usize,
    materialized_nodes: usize,
    materialized_bytes: usize,
    lowering_depth: usize,
    default_validation_steps: usize,
}

pub(super) fn compile<'names>(
    bundle: &Bundle,
    document: &Document,
    expected: Option<&str>,
    additional_types: impl IntoIterator<Item = &'names str>,
) -> Result<Schemas, NativeError> {
    let mut compiler = Compiler {
        bundle,
        contexts: HashMap::new(),
        schema_sources: HashMap::new(),
        active: HashSet::new(),
        source_bytes: bundle.source_bytes,
        types: Map::new(),
        resources: Vec::new(),
        registrations: HashMap::new(),
        builtin_tags: HashSet::new(),
        checks: Vec::new(),
        regex_origins: HashMap::new(),
        next_regex_origin: 0,
        default_projections: HashMap::new(),
        next_default_projection: 0,
        materialized_nodes: 0,
        materialized_bytes: 0,
        lowering_depth: 0,
        default_validation_steps: 0,
    };
    let mut roots = Vec::new();
    collect_config_tags(document, document.root(), &mut roots);
    if let Some(expected) = expected {
        roots.push((expected, document.root().span));
    }
    let mut source_tags = roots
        .iter()
        .map(|(tag, _)| (*tag).to_owned())
        .collect::<HashSet<_>>();
    for &(tag, span) in &roots {
        if !compiler.registrations.contains_key(tag) {
            let (key, _) = compiler.load_tag(tag, Some(span), &bundle.root)?;
            compiler.registrations.insert(tag.to_owned(), key);
        }
    }
    let scalar_limit = crate::Limits::default().max_scalar_bytes;
    for (index, tag) in additional_types.into_iter().enumerate() {
        if index >= crate::schema::profile::MAX_SCHEMA_NODES || tag.len() > scalar_limit {
            return Err(NativeError::new(
                NativeErrorKind::LimitExceeded,
                "additional native type names exceed the compiler input bound",
            )
            .at_path(&bundle.root));
        }
        admit_copy_cost(
            &mut compiler.materialized_nodes,
            &mut compiler.materialized_bytes,
            1,
            tag.len(),
            &bundle.root,
            None,
        )?;
        if !compiler.registrations.contains_key(tag) {
            let (key, _) = compiler.load_tag(tag, None, &bundle.root)?;
            compiler.registrations.insert(tag.to_owned(), key);
            source_tags.insert(tag.to_owned());
        }
    }
    source_tags.extend(compiler.builtin_tags.iter().cloned());
    for tag in &compiler.builtin_tags {
        let key = format!("native_builtin_{tag}");
        if compiler.types.contains_key(&key) {
            compiler.registrations.entry(tag.clone()).or_insert(key);
        }
    }
    for (alias, key) in compiler.registrations {
        if alias != key && !compiler.types.contains_key(&alias) {
            let target = compiler
                .types
                .get(&key)
                .and_then(|entry| entry["schema"]["$id"].as_str())
                .expect("registered native binding has a schema identifier")
                .to_owned();
            let reference = Json::Object(Map::from_iter([("$ref".into(), Json::String(target))]));
            let binding = Json::Object(Map::from_iter([("schema".into(), reference)]));
            compiler.types.insert(alias, binding);
        }
    }
    let validation_schemas = if compiler.checks.is_empty() {
        None
    } else {
        admit_copy_cost(
            &mut compiler.materialized_nodes,
            &mut compiler.materialized_bytes,
            2,
            0,
            &bundle.root,
            None,
        )?;
        for (key, binding) in &compiler.types {
            admit_copy(
                &mut compiler.materialized_nodes,
                &mut compiler.materialized_bytes,
                binding,
                &bundle.root,
                None,
            )?;
            admit_copy_cost(
                &mut compiler.materialized_nodes,
                &mut compiler.materialized_bytes,
                1,
                key.len(),
                &bundle.root,
                None,
            )?;
        }
        for resource in &compiler.resources {
            admit_copy(
                &mut compiler.materialized_nodes,
                &mut compiler.materialized_bytes,
                resource,
                &bundle.root,
                None,
            )?;
        }
        let (validation_types, validation_resources) =
            relaxed_projection(&compiler.types, &compiler.resources);
        let validation_value = Json::Object(Map::from_iter([
            ("types".into(), Json::Object(validation_types)),
            ("resources".into(), Json::Array(validation_resources)),
        ]));
        let validation_module = SchemaModule::from_value(validation_value)
            .map_err(|error| native_schema_error(error, &bundle.root, &compiler.regex_origins))?;
        Some(
            Schemas::compose([validation_module]).map_err(|error| {
                native_schema_error(error, &bundle.root, &compiler.regex_origins)
            })?,
        )
    };
    let module_value = Json::Object(Map::from_iter([
        ("types".into(), Json::Object(compiler.types)),
        ("resources".into(), Json::Array(compiler.resources)),
    ]));
    let module = SchemaModule::from_value(module_value)
        .map_err(|error| native_schema_error(error, &bundle.root, &compiler.regex_origins))?;
    let mut schemas = Schemas::compose([module])
        .map_err(|error| native_schema_error(error, &bundle.root, &compiler.regex_origins))?;
    schemas.restrict_source_tags(source_tags);
    let mut default_limits = crate::ResolutionLimits::default();
    for check in compiler.checks {
        admit_copy(
            &mut compiler.materialized_nodes,
            &mut compiler.materialized_bytes,
            &check.value,
            &check.path,
            Some(check.span),
        )?;
        for annotation in &check.annotations {
            admit_copy(
                &mut compiler.materialized_nodes,
                &mut compiler.materialized_bytes,
                annotation,
                &check.path,
                Some(check.span),
            )?;
        }
        let location = crate::SchemaLocation {
            resource: format!("file://{}", check.path.display()),
            pointer: "/default".into(),
        };
        let node = resolved_node(&check.value, &check.annotations, &location, "", check.span)?;
        let validation = validation_schemas
            .as_ref()
            .expect("default checks have a validation projection");
        validation
            .complete_partial_default(&check.schema_type, node, &mut default_limits)
            .map_err(|error| {
                let kind = if matches!(
                    error.kind,
                    crate::ResolveErrorKind::LimitExceeded | crate::ResolveErrorKind::NumericLimit
                ) {
                    NativeErrorKind::LimitExceeded
                } else {
                    NativeErrorKind::InvalidDefault
                };
                NativeError::new(
                    kind,
                    format!("default violates its declared constraints: {error}"),
                )
                .caused_by(error)
                .at_path(&check.path)
                .at_span(check.span)
            })?;
    }
    Ok(schemas)
}
fn native_schema_error(
    error: SchemaError,
    fallback: &Path,
    origins: &HashMap<String, RegexOrigin>,
) -> NativeError {
    let origin = error
        .location
        .as_ref()
        .and_then(|location| origins.get(&location.resource));
    let mut native = NativeError::new(NativeErrorKind::Schema, error.to_string()).caused_by(error);
    if let Some(origin) = origin {
        native = native.at_path(&origin.path).at_span(origin.span);
    } else {
        native = native.at_path(fallback);
    }
    native
}

// Keep registry IDs intact; this default-only view treats required members and minimum object counts as presence.
fn relaxed_projection(
    types: &Map<String, Json>,
    resources: &[Json],
) -> (Map<String, Json>, Vec<Json>) {
    let mut types = types.clone();
    for binding in types.values_mut() {
        if let Some(schema) = binding.get_mut("schema") {
            relax_schema(schema);
        }
    }
    let mut resources = resources.to_vec();
    for resource in &mut resources {
        if let Some(schema) = resource.get_mut("schema") {
            relax_schema(schema);
        }
    }
    (types, resources)
}

fn relax_schema(schema: &mut Json) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    object.remove("required");
    object.remove("minProperties");
    crate::schema::profile::for_each_child_mut(schema, relax_schema);
}
fn collect_config_tags<'a>(document: &'a Document, node: &Node, tags: &mut Vec<(&'a str, Span)>) {
    if let Some(tag) = document.tag(node) {
        tags.push((tag, node.tag.expect("tag has source span")));
    }
    match &node.value {
        Value::Sequence(items) => {
            for item in items {
                collect_config_tags(document, item, tags);
            }
        }
        Value::Mapping(entries) => {
            for entry in entries {
                collect_config_tags(document, &entry.value, tags);
            }
        }
        _ => {}
    }
}

impl Compiler<'_> {
    fn load_tag(
        &mut self,
        tag: &str,
        span: Option<Span>,
        context_root: &Path,
    ) -> Result<(String, String), NativeError> {
        if let Some(schema) = builtin(tag) {
            self.builtin_tags.insert(tag.to_owned());
            let key = format!("native_builtin_{tag}");
            let id = format!("urn:daml:native:builtin:{tag}");
            if !self.types.contains_key(&key) {
                let mut schema = Json::Object(schema);
                schema
                    .as_object_mut()
                    .expect("JSON object")
                    .insert("$id".into(), Json::String(id.clone()));
                self.types.insert(
                    key.clone(),
                    Json::Object(Map::from_iter([("schema".into(), schema)])),
                );
            }
            self.schema_sources
                .insert(id.clone(), SchemaSource::Type(key.clone()));
            return Ok((key, id));
        }
        if tag == "Never" {
            self.builtin_tags.insert(tag.to_owned());
            let key = "native_builtin_Never".to_owned();
            let id = "urn:daml:native:builtin:Never".to_owned();
            self.types
                .entry(key.clone())
                .or_insert_with(|| serde_json::json!({"schema":{"$id":id,"not":{}}}));
            self.schema_sources
                .insert(id.clone(), SchemaSource::Type(key.clone()));
            return Ok((key, id));
        }
        let (root, relative) = self.locate(tag, span, context_root)?;
        let canonical_root = self
            .bundle
            .canonical_root(&root)
            .map_err(|error| error.at_span(span).for_reference(tag))?;
        self.imports_for(&canonical_root)?;
        let file = canonical_root.join(relative);
        let mut logical_parent = file.parent();
        while let Some(directory) = logical_parent {
            if directory == canonical_root.as_path() {
                break;
            }
            if self.bundle.has_manifest(directory) {
                return Err(NativeError::new(
                    NativeErrorKind::NestedBundleBoundary,
                    "definition route crosses a nested bundle; add it as an explicit import",
                )
                .at_path(directory)
                .at_span(span)
                .for_reference(tag));
            }
            logical_parent = directory.parent();
        }
        let canonical_file = self
            .bundle
            .canonical_file(&file)
            .map_err(|error| error.at_span(span).for_reference(tag))?;
        if !canonical_file.starts_with(&canonical_root) {
            return Err(NativeError::new(
                NativeErrorKind::PathEscape,
                "definition resolves outside selected bundle root",
            )
            .at_path(canonical_file)
            .at_span(span)
            .for_reference(tag));
        }
        let mut parent = canonical_file.parent();
        while let Some(directory) = parent {
            if directory == canonical_root.as_path() {
                break;
            }
            if self.bundle.has_manifest(directory) {
                return Err(NativeError::new(NativeErrorKind::NestedBundleBoundary, "definition path crosses a nested bundle; add the nested bundle as an explicit import").at_path(directory).at_span(span).for_reference(tag));
            }
            parent = directory.parent();
        }
        let relative = canonical_file
            .strip_prefix(&canonical_root)
            .expect("checked root");
        let identity = definition_identity(&canonical_root, relative);
        let key = format!("native_{identity}");
        let id = format!("urn:daml:native:{identity}");
        if self.types.contains_key(&key) {
            return Ok((key, id));
        }
        if self.active.len() >= crate::schema::profile::MAX_SCHEMA_DEPTH {
            return Err(NativeError::new(
                NativeErrorKind::LimitExceeded,
                "reachable native reference depth exceeds schema-depth bound",
            )
            .at_path(&canonical_file)
            .at_span(span)
            .for_reference(tag));
        }
        if self.types.len() >= crate::schema::profile::MAX_SCHEMA_NODES {
            return Err(NativeError::new(
                NativeErrorKind::LimitExceeded,
                "reachable native definition count exceeds schema-node bound",
            )
            .at_path(&canonical_file));
        }
        let remaining = crate::schema::profile::MAX_SOURCE_BYTES.saturating_sub(self.source_bytes);
        if !self.active.insert(key.clone()) {
            return Err(NativeError::new(
                NativeErrorKind::ReferenceCycle,
                "definition reference cycle is unsupported",
            )
            .at_path(&canonical_file)
            .at_span(span)
            .for_reference(tag));
        }
        let source = self
            .bundle
            .read_resource(&canonical_file, remaining)
            .map_err(|error| error.at_span(span).for_reference(tag))?;
        self.source_bytes += source.len();
        let definition = super::parse_definition(source).map_err(|error| {
            NativeError::new(NativeErrorKind::InvalidDefinition, error.to_string())
                .at_path(&canonical_file)
                .at_span(error.span)
                .caused_by(error)
        })?;
        let (mut contract, mut full) = self.lower_definition(
            &definition.root,
            definition.source(),
            &canonical_root,
            &canonical_file,
        )?;
        let contract_id = format!("{id}:contract");
        contract.insert("$id".into(), Json::String(contract_id.clone()));
        let contract_index = self.resources.len();
        self.resources.push(Json::Object(Map::from_iter([
            ("uri".into(), Json::String(contract_id.clone())),
            ("schema".into(), Json::Object(contract)),
        ])));
        self.schema_sources
            .insert(contract_id, SchemaSource::Resource(contract_index));
        full.insert("$id".into(), Json::String(id.clone()));
        self.types.insert(
            key.clone(),
            Json::Object(Map::from_iter([("schema".into(), Json::Object(full))])),
        );
        self.schema_sources
            .insert(id.clone(), SchemaSource::Type(key.clone()));
        self.active.remove(&key);
        Ok((key, id))
    }

    fn locate(
        &mut self,
        tag: &str,
        span: Option<Span>,
        context_root: &Path,
    ) -> Result<(PathBuf, PathBuf), NativeError> {
        let mut parts = tag.split('.');
        let first = parts.next().unwrap_or_default();
        let root = if first == "self" {
            context_root.to_path_buf()
        } else {
            self.imports_for(context_root)?;
            let imports = if context_root == self.bundle.root.as_path() {
                &self.bundle.imports
            } else {
                self.contexts.get(context_root).ok_or_else(|| {
                    NativeError::new(
                        NativeErrorKind::InvalidManifest,
                        "selected bundle manifest was not admitted",
                    )
                    .at_path(context_root.join(super::BUNDLE_MANIFEST))
                })?
            };
            imports.get(first).cloned().ok_or_else(|| {
                NativeError::new(
                    NativeErrorKind::UnknownReference,
                    format!("unknown bundle alias in tag {tag:?}"),
                )
                .at_span(span)
                .for_reference(tag)
            })?
        };
        let path = parts.collect::<Vec<_>>();
        if path.is_empty() || path.iter().any(|part| !crate::names::valid_segment(part)) {
            return Err(NativeError::new(
                NativeErrorKind::InvalidName,
                format!("invalid definition path in tag {tag:?}"),
            )
            .at_span(span)
            .for_reference(tag));
        }
        let mut relative = PathBuf::new();
        for segment in path {
            relative.push(segment);
        }
        relative.set_extension("daml");
        Ok((root, relative))
    }

    fn imports_for(&mut self, root: &Path) -> Result<(), NativeError> {
        if root == self.bundle.root.as_path() || self.contexts.contains_key(root) {
            return Ok(());
        }
        let canonical = self.bundle.canonical_root(root)?;
        if canonical == self.bundle.root || self.contexts.contains_key(&canonical) {
            return Ok(());
        }
        let (canonical, imports, source_bytes) = self.bundle.load_imports(&canonical)?;
        if source_bytes > crate::schema::profile::MAX_SOURCE_BYTES.saturating_sub(self.source_bytes)
        {
            return Err(NativeError::new(
                NativeErrorKind::LimitExceeded,
                "reachable bundle manifests exceed aggregate source-byte bound",
            )
            .at_path(canonical.join(super::BUNDLE_MANIFEST)));
        }
        self.source_bytes += source_bytes;
        self.contexts.insert(canonical, imports);
        Ok(())
    }

    fn project_default_reference(
        &mut self,
        reference: &str,
        path: &[DefaultPathStep],
        file: &Path,
    ) -> Result<String, NativeError> {
        let mut budget = None;
        self.project_default_reference_inner(
            reference,
            path,
            file,
            &mut HashSet::new(),
            &mut budget,
        )
    }
    fn project_default_reference_inner(
        &mut self,
        reference: &str,
        path: &[DefaultPathStep],
        file: &Path,
        visited: &mut HashSet<(String, DefaultPath)>,
        budget: &mut Option<&mut WalkBudget<'_>>,
    ) -> Result<String, NativeError> {
        let key = (reference.to_owned(), path.to_vec());
        if let Some(projected) = self.default_projections.get(&key) {
            return Ok(projected.clone());
        }
        if !visited.insert(key.clone()) {
            return Ok(reference.to_owned());
        }
        let Some(source) = self.schema_sources.get(reference).cloned() else {
            self.default_projections
                .insert(key.clone(), reference.to_owned());
            visited.remove(&key);
            return Ok(reference.to_owned());
        };
        let source_schema = match &source {
            SchemaSource::Type(key) => self.types.get(key).and_then(|entry| entry.get("schema")),
            SchemaSource::Resource(index) => self
                .resources
                .get(*index)
                .and_then(|entry| entry.get("schema")),
        };
        let Some(source_schema) = source_schema else {
            self.default_projections
                .insert(key.clone(), reference.to_owned());
            visited.remove(&key);
            return Ok(reference.to_owned());
        };
        if let Some(walk) = budget.as_deref_mut() {
            walk.charge_copy(source_schema)?;
        } else {
            admit_copy(
                &mut self.materialized_nodes,
                &mut self.materialized_bytes,
                source_schema,
                file,
                None,
            )?;
        }
        let mut schema = source_schema.clone();
        if !self.project_schema_defaults(&mut schema, path, file, visited, budget)? {
            visited.remove(&key);
            self.default_projections.insert(key, reference.to_owned());
            return Ok(reference.to_owned());
        }
        if self.next_default_projection >= crate::schema::profile::MAX_SCHEMA_NODES
            || self.types.len() >= crate::schema::profile::MAX_SCHEMA_NODES
        {
            return Err(NativeError::new(
                NativeErrorKind::LimitExceeded,
                "native default projection exceeds schema-node bound",
            )
            .at_path(file));
        }
        let variant = self.next_default_projection;
        self.next_default_projection += 1;
        let name = format!("native_default_projection_{variant}");
        let id = format!("urn:daml:native:default-projection:{variant}");
        schema
            .as_object_mut()
            .expect("registered schema is an object")
            .insert("$id".into(), Json::String(id.clone()));
        match source {
            SchemaSource::Type(_) => {
                self.types.insert(
                    name.clone(),
                    Json::Object(Map::from_iter([("schema".into(), schema)])),
                );
                self.schema_sources
                    .insert(id.clone(), SchemaSource::Type(name));
            }
            SchemaSource::Resource(_) => {
                let index = self.resources.len();
                self.resources.push(Json::Object(Map::from_iter([
                    ("uri".into(), Json::String(id.clone())),
                    ("schema".into(), schema),
                ])));
                self.schema_sources
                    .insert(id.clone(), SchemaSource::Resource(index));
            }
        }
        self.default_projections.insert(key.clone(), id.clone());
        visited.remove(&key);
        Ok(id)
    }

    fn project_schema_defaults(
        &mut self,
        schema: &mut Json,
        path: &[DefaultPathStep],
        file: &Path,
        visited: &mut HashSet<(String, DefaultPath)>,
        budget: &mut Option<&mut WalkBudget<'_>>,
    ) -> Result<bool, NativeError> {
        if !schema.is_object() {
            return Ok(false);
        }
        let mut changed = false;
        let reference = {
            let object = schema.as_object_mut().expect("checked object");
            if path.is_empty() {
                changed |= object.remove("default").is_some();
                changed |= object.remove("x-daml-default-tags").is_some();
                changed |= object.remove("x-daml-default-temporals").is_some();
            } else {
                if let Some(default) = object.get_mut("default") {
                    changed |= remove_default_component(default, path);
                }
                let pointer = default_path_pointer(path);
                let prefix = format!("{pointer}/");
                for keyword in ["x-daml-default-tags", "x-daml-default-temporals"] {
                    if let Some(annotations) = object.get_mut(keyword).and_then(Json::as_array_mut)
                    {
                        let prior = annotations.len();
                        annotations.retain(|annotation| {
                            annotation
                                .get("path")
                                .and_then(Json::as_str)
                                .is_none_or(|path| path != pointer && !path.starts_with(&prefix))
                        });
                        changed |= annotations.len() != prior;
                        if annotations.is_empty() {
                            object.remove(keyword);
                        }
                    }
                }
            }
            object.get("$ref").and_then(Json::as_str).map(str::to_owned)
        };
        if let Some(reference) = reference {
            let projected =
                self.project_default_reference_inner(&reference, path, file, visited, budget)?;
            if projected != reference {
                schema
                    .as_object_mut()
                    .expect("checked object")
                    .insert("$ref".into(), Json::String(projected));
                changed = true;
            }
        }
        let homogeneous_index = match path.first() {
            Some(DefaultPathStep::Item(index)) => Some(*index),
            _ => None,
        };
        if let Some(index) = homogeneous_index
            && index
                >= schema
                    .get("prefixItems")
                    .and_then(Json::as_array)
                    .map_or(0, Vec::len)
            && let Some(items) = schema.get("items")
        {
            if let Some(walk) = budget.as_deref_mut() {
                walk.charge_copy(items)?;
            } else {
                admit_copy(
                    &mut self.materialized_nodes,
                    &mut self.materialized_bytes,
                    items,
                    file,
                    None,
                )?;
            }
            let items = items.clone();
            let prefix = schema
                .as_object_mut()
                .expect("checked object")
                .entry("prefixItems")
                .or_insert_with(|| Json::Array(Vec::new()))
                .as_array_mut()
                .expect("native prefixItems is an array");
            while prefix.len() <= index {
                let child = items.clone();
                if let Some(walk) = budget.as_deref_mut() {
                    walk.charge_copy(&child)?;
                } else {
                    admit_copy(
                        &mut self.materialized_nodes,
                        &mut self.materialized_bytes,
                        &child,
                        file,
                        None,
                    )?;
                }
                prefix.push(child);
            }
            changed = true;
        }
        let additional_property = match path.first() {
            Some(DefaultPathStep::Property(key)) => (schema
                .get("properties")
                .and_then(Json::as_object)
                .is_none_or(|properties| !properties.contains_key(key))
                && schema.get("patternProperties").is_none())
            .then(|| {
                schema
                    .get("additionalProperties")
                    .map(|child| (key.clone(), child))
            })
            .flatten(),
            _ => None,
        };
        if let Some((key, child)) = additional_property {
            if let Some(walk) = budget.as_deref_mut() {
                walk.charge_copy(child)?;
            } else {
                admit_copy(
                    &mut self.materialized_nodes,
                    &mut self.materialized_bytes,
                    child,
                    file,
                    None,
                )?;
            }
            let mut projected = child.clone();
            if self.project_schema_defaults(&mut projected, &path[1..], file, visited, budget)? {
                let object = schema.as_object_mut().expect("checked object");
                let properties = object
                    .entry("properties")
                    .or_insert_with(|| Json::Object(Map::new()))
                    .as_object_mut()
                    .expect("native projected properties are an object");
                properties.insert(key, projected);
                changed = true;
            }
        }
        let mut nested_error = None;
        crate::schema::profile::for_each_child_mut_with_location(
            schema,
            |keyword, map_key, array_index, _applies, _unsupported_defaults, child| {
                let child_path = match keyword {
                    "allOf" | "anyOf" => Some(path),
                    "properties" => map_key
                        .filter(|key| matches!(path.first(), Some(DefaultPathStep::Property(segment)) if segment == *key))
                        .map(|_| &path[1..]),
                    "prefixItems" => array_index
                        .filter(|index| matches!(path.first(), Some(DefaultPathStep::Item(position)) if position == index))
                        .map(|_| &path[1..]),
                    _ => None,
                };
                if let Some(child_path) = child_path {
                    match self.project_schema_defaults(child, child_path, file, visited, budget) {
                        Ok(child_changed) => changed |= child_changed,
                        Err(error) => nested_error = Some(error),
                    }
                }
            },
        );
        if let Some(error) = nested_error {
            return Err(error);
        }
        Ok(changed)
    }
    fn lower_definition(
        &mut self,
        expression: &crate::native::Expression,
        source: &str,
        root: &Path,
        file: &Path,
    ) -> Result<(SchemaObject, SchemaObject), NativeError> {
        let context = SourceContext { source, root, file };
        let full = self.lower_expression(expression, context, false, true)?;
        let mut full_schema = Json::Object(full);
        self.validate_effective_defaults(&mut full_schema, file, expression.span)?;
        let Json::Object(full) = full_schema else {
            unreachable!("native definition lowering always produces a schema object");
        };
        let contract = self.lower_expression(expression, context, true, false)?;
        Ok((contract, full))
    }
    fn materialize_node(
        &mut self,
        source: &str,
        node: &Node,
        file: &Path,
    ) -> Result<Json, NativeError> {
        let (nodes, bytes) = node_json_cost(source, node);
        admit_copy_cost(
            &mut self.materialized_nodes,
            &mut self.materialized_bytes,
            nodes,
            bytes,
            file,
            Some(node.span),
        )?;
        node_json(source, node)
    }

    fn lower_metadata(
        &mut self,
        name: &str,
        body: &crate::native::Expression,
        source: &str,
        root: &Path,
        file: &Path,
        check_defaults: bool,
    ) -> Result<Map<String, Json>, NativeError> {
        use crate::native::ExpressionKind as K;
        let K::Record(members) = &body.kind else {
            return Err(NativeError::new(
                NativeErrorKind::InvalidDefinition,
                format!("{name} metadata must be a record"),
            )
            .at_path(file)
            .at_span(body.span));
        };
        let mut schema = Map::new();
        for member in members {
            let key = source.get(member.key.start..member.key.end).unwrap_or("");
            let value_constraint = matches!((name, key), ("List", "items") | ("Map", "values"));
            if member.optional
                || (!value_constraint && has_default_in_expression_tree(&member.constraint))
            {
                return Err(NativeError::new(
                    NativeErrorKind::InvalidDefinition,
                    "metadata cannot be optional; only item/value constraints may have defaults",
                )
                .at_path(file)
                .at_span(member.span));
            }
            match (name, key) {
                ("Time" | "DateTime", "offset") => {
                    let K::Literal(node) = &member.constraint.kind else {
                        return Err(NativeError::new(
                            NativeErrorKind::InvalidDefinition,
                            "offset policy requires a string literal",
                        )
                        .at_path(file)
                        .at_span(member.constraint.span));
                    };
                    let policy = self.materialize_node(source, node, file)?;
                    if !matches!(policy.as_str(), Some("optional" | "required" | "forbidden")) {
                        return Err(NativeError::new(
                            NativeErrorKind::InvalidDefinition,
                            "offset policy must be optional, required or forbidden",
                        )
                        .at_path(file)
                        .at_span(node.span));
                    }
                    let kind = if name == "Time" { "time" } else { "datetime" };
                    schema.insert(
                        "x-daml-scalar".into(),
                        serde_json::json!({"kind":kind,"offset":policy}),
                    );
                }
                (
                    "Int" | "Number",
                    "min" | "max" | "exclusiveMin" | "exclusiveMax" | "multipleOf",
                ) => {
                    let K::Literal(value) = &member.constraint.kind else {
                        return Err(NativeError::new(
                            NativeErrorKind::InvalidDefinition,
                            format!("{name} facet {key:?} requires a numeric literal"),
                        )
                        .at_path(file)
                        .at_span(member.constraint.span));
                    };
                    let Value::Number(_) = &value.value else {
                        return Err(NativeError::new(
                            NativeErrorKind::InvalidDefinition,
                            format!("{name} facet {key:?} requires a numeric literal"),
                        )
                        .at_path(file)
                        .at_span(value.span));
                    };
                    let bound = self.materialize_node(source, value, file)?;
                    if key == "multipleOf" {
                        let Json::Number(number) = &bound else {
                            unreachable!("numeric literal materializes as a number");
                        };
                        if !is_positive_number_literal(number.as_str()) {
                            return Err(NativeError::new(
                                NativeErrorKind::InvalidDefinition,
                                format!("{name} facet \"multipleOf\" requires a positive number"),
                            )
                            .at_path(file)
                            .at_span(value.span));
                        }
                    }
                    let keyword = match key {
                        "min" => "minimum",
                        "max" => "maximum",
                        "exclusiveMin" => "exclusiveMinimum",
                        "exclusiveMax" => "exclusiveMaximum",
                        "multipleOf" => "multipleOf",
                        _ => unreachable!("matched numeric facet"),
                    };
                    schema.insert(keyword.into(), bound);
                }
                ("String" | "List" | "Map", "min" | "max") => {
                    let bound = count_facet_bound(name, key, &member.constraint, source, file)?;
                    let keyword = match (name, key) {
                        ("String", "min") => "minLength",
                        ("String", "max") => "maxLength",
                        ("List", "min") => "minItems",
                        ("List", "max") => "maxItems",
                        ("Map", "min") => "minProperties",
                        ("Map", "max") => "maxProperties",
                        _ => unreachable!("matched local count facet"),
                    };
                    schema.insert(keyword.into(), Json::Number(Number::from(bound)));
                }
                ("String", "pattern") => {
                    let K::Regex(pattern) = &member.constraint.kind else {
                        return Err(NativeError::new(
                            NativeErrorKind::InvalidDefinition,
                            "String facet \"pattern\" requires a Regex",
                        )
                        .at_path(file)
                        .at_span(member.constraint.span));
                    };
                    let origin = self.next_regex_origin;
                    self.next_regex_origin = self
                        .next_regex_origin
                        .checked_add(1)
                        .expect("Regex origins are bounded by schema nodes");
                    self.regex_origins.insert(
                        crate::schema::native_regex_origin(origin),
                        RegexOrigin {
                            path: file.to_path_buf(),
                            span: *pattern,
                        },
                    );
                    schema.insert(
                        "x-daml-regex".into(),
                        Json::Object(Map::from_iter([
                            (
                                "pattern".into(),
                                Json::String(
                                    source
                                        .get(pattern.start..pattern.end)
                                        .unwrap_or("")
                                        .to_owned(),
                                ),
                            ),
                            ("origin".into(), Json::Number(Number::from(origin))),
                        ])),
                    );
                }
                ("List", "unique") => {
                    let K::Literal(value) = &member.constraint.kind else {
                        return Err(NativeError::new(
                            NativeErrorKind::InvalidDefinition,
                            "List facet \"unique\" must be true",
                        )
                        .at_path(file)
                        .at_span(member.constraint.span));
                    };
                    if !matches!(value.value, Value::Bool(true)) {
                        return Err(NativeError::new(
                            NativeErrorKind::InvalidDefinition,
                            "List facet \"unique\" must be true",
                        )
                        .at_path(file)
                        .at_span(value.span));
                    }
                    schema.insert("uniqueItems".into(), Json::Bool(true));
                }
                ("List", "items") | ("Map", "values") => {
                    let lowered = self.lower_expression(
                        &member.constraint,
                        SourceContext { source, root, file },
                        false,
                        check_defaults,
                    )?;
                    schema.insert(
                        if name == "List" {
                            "items"
                        } else {
                            "additionalProperties"
                        }
                        .into(),
                        Json::Object(lowered),
                    );
                }
                _ => {
                    return Err(NativeError::new(
                        NativeErrorKind::InvalidDefinition,
                        format!("unknown {name} metadata field {key:?}"),
                    )
                    .at_path(file)
                    .at_span(member.key));
                }
            }
        }
        for (inclusive, exclusive) in [
            ("minimum", "exclusiveMinimum"),
            ("maximum", "exclusiveMaximum"),
        ] {
            if schema.contains_key(inclusive) && schema.contains_key(exclusive) {
                let (inclusive, exclusive) = match inclusive {
                    "minimum" => ("min", "exclusiveMin"),
                    "maximum" => ("max", "exclusiveMax"),
                    _ => unreachable!("matched numeric bound"),
                };
                return Err(NativeError::new(
                    NativeErrorKind::InvalidDefinition,
                    format!("{name} cannot define both {inclusive:?} and {exclusive:?}"),
                )
                .at_path(file)
                .at_span(body.span));
            }
        }
        if check_defaults {
            let mut schema = Json::Object(schema);
            self.validate_effective_defaults(&mut schema, file, body.span)?;
            let Json::Object(schema) = schema else {
                unreachable!("metadata lowering always produces a schema object");
            };
            return Ok(schema);
        }
        Ok(schema)
    }

    fn lower_intersection(
        &mut self,
        items: &[crate::native::Expression],
        context: SourceContext<'_>,
        suppress_children: bool,
        validate_children: bool,
    ) -> Result<Vec<Json>, NativeError> {
        use crate::native::ExpressionKind as K;
        let mut tuple_overrides = Vec::new();
        let mut expressions: Vec<_> = items.iter().collect();
        while let Some(item) = expressions.pop() {
            match &item.kind {
                K::Tagged {
                    name,
                    body: Some(body),
                } if context.source.get(name.start..name.end) == Some("Tuple") => {
                    let K::Tuple(positions) = &body.kind else {
                        continue;
                    };
                    let mut prefix = Vec::new();
                    for (index, position) in positions.iter().enumerate() {
                        prefix.push(DefaultPathStep::Item(index));
                        collect_default_override_paths(
                            position,
                            context.source,
                            &mut prefix,
                            &mut tuple_overrides,
                        );
                        prefix.pop();
                    }
                }
                K::Group(inner) | K::Closed(inner)
                    if matches!(
                        item.effective_default(),
                        crate::native::DefaultValue::Inherited
                    ) =>
                {
                    expressions.push(inner);
                }
                K::Intersection(children)
                    if matches!(
                        item.effective_default(),
                        crate::native::DefaultValue::Inherited
                    ) =>
                {
                    expressions.extend(children);
                }
                _ => {}
            }
        }

        let mut branches = Vec::with_capacity(items.len());
        let mut base_schemas = Vec::new();
        for item in items {
            let mut branch = Json::Object(self.lower_expression(
                item,
                context,
                suppress_children,
                validate_children,
            )?);
            if !tuple_overrides.is_empty() {
                visit_conjunction_leaves_mut(item, &mut branch, &mut |operand, schema| {
                    if let K::Tagged { body: None, .. } = &operand.kind
                        && let Some(reference) = schema.get("$ref").and_then(Json::as_str)
                    {
                        if let Some(base) =
                            self.schema_sources
                                .get(reference)
                                .and_then(|source| match source {
                                    SchemaSource::Type(key) => {
                                        self.types.get(key).and_then(|entry| entry.get("schema"))
                                    }
                                    SchemaSource::Resource(index) => self
                                        .resources
                                        .get(*index)
                                        .and_then(|entry| entry.get("schema")),
                                })
                        {
                            admit_copy(
                                &mut self.materialized_nodes,
                                &mut self.materialized_bytes,
                                base,
                                context.file,
                                Some(operand.span),
                            )?;
                            base_schemas.push(base.clone());
                        }
                        let mut projected = reference.to_owned();
                        for path in &tuple_overrides {
                            projected =
                                self.project_default_reference(&projected, path, context.file)?;
                        }
                        schema
                            .as_object_mut()
                            .expect("reference lowering produces an object schema")
                            .insert("$ref".into(), Json::String(projected));
                    }
                    Ok(())
                })?;
            }
            branches.push(branch);
        }
        if !base_schemas.is_empty() {
            for (item, branch) in items.iter().zip(&mut branches) {
                visit_conjunction_leaves_mut(item, branch, &mut |operand, schema| {
                    if let K::Tagged {
                        name,
                        body: Some(_),
                    } = &operand.kind
                        && context.source.get(name.start..name.end) == Some("Tuple")
                    {
                        self.normalize_conditional_default_overrides_from_schemas(
                            &base_schemas,
                            schema,
                            operand,
                            context.source,
                            context.file,
                        )?;
                    }
                    Ok(())
                })?;
            }
        }
        Ok(branches)
    }

    fn lower_tagged(
        &mut self,
        expression: &crate::native::Expression,
        context: SourceContext<'_>,
        suppress_children: bool,
        validate_children: bool,
    ) -> Result<Map<String, Json>, NativeError> {
        let crate::native::ExpressionKind::Tagged { name, body } = &expression.kind else {
            unreachable!("tagged lowering receives a tagged expression");
        };
        let name = *name;
        let body = body.as_deref();
        let tag = context.source.get(name.start..name.end).unwrap_or("");
        if let Some(kind) = builtin(tag) {
            self.builtin_tags.insert(tag.to_owned());
            return self.lower_builtin_tag(
                expression,
                tag,
                kind,
                context,
                suppress_children,
                validate_children,
            );
        }
        if tag == "Never" {
            self.builtin_tags.insert(tag.to_owned());
            if body.is_some() {
                return Err(NativeError::new(
                    NativeErrorKind::InvalidDefinition,
                    "Never does not accept metadata",
                )
                .at_path(context.file)
                .at_span(name));
            }
            return Ok(Map::from_iter([("not".into(), Json::Object(Map::new()))]));
        }
        self.lower_reference_tag(
            expression,
            tag,
            context,
            suppress_children,
            validate_children,
        )
    }

    fn lower_builtin_tag(
        &mut self,
        expression: &crate::native::Expression,
        tag: &str,
        mut schema: Map<String, Json>,
        context: SourceContext<'_>,
        suppress_children: bool,
        validate_children: bool,
    ) -> Result<Map<String, Json>, NativeError> {
        let crate::native::ExpressionKind::Tagged { name, body } = &expression.kind else {
            unreachable!("builtin lowering receives a tagged expression");
        };
        let name = *name;
        let body = body.as_deref();
        if body.is_some() && matches!(tag, "Bool" | "Any") {
            return Err(NativeError::new(
                NativeErrorKind::InvalidDefinition,
                format!("{tag} does not accept metadata"),
            )
            .at_path(context.file)
            .at_span(name));
        }
        if tag == "Tuple" {
            schema.extend(self.lower_tuple_tag(name, body, context, validate_children)?);
        } else if let Some(body) = body {
            if matches!(
                tag,
                "String" | "Int" | "Number" | "List" | "Map" | "Date" | "Time" | "DateTime"
            ) {
                schema.extend(self.lower_metadata(
                    tag,
                    body,
                    context.source,
                    context.root,
                    context.file,
                    validate_children,
                )?);
            } else {
                let narrowed =
                    self.lower_expression(body, context, suppress_children, validate_children)?;
                if !narrowed.is_empty() {
                    schema.insert("allOf".into(), Json::Array(vec![Json::Object(narrowed)]));
                }
            }
        }
        Ok(schema)
    }

    fn lower_reference_tag(
        &mut self,
        expression: &crate::native::Expression,
        tag: &str,
        context: SourceContext<'_>,
        suppress_children: bool,
        validate_children: bool,
    ) -> Result<Map<String, Json>, NativeError> {
        let crate::native::ExpressionKind::Tagged { name, body } = &expression.kind else {
            unreachable!("reference lowering receives a tagged expression");
        };
        let name = *name;
        let body = body.as_deref();
        let SourceContext { source, root, file } = context;
        let (_, id) = self.load_tag(tag, Some(name), root).map_err(|error| {
            if error.path.is_none() {
                error.at_path(file)
            } else {
                error
            }
        })?;
        let base = if body.is_some_and(supports_default_override_projection) {
            let base = self
                .schema_sources
                .get(&id)
                .and_then(|source| match source {
                    SchemaSource::Type(key) => {
                        self.types.get(key).and_then(|entry| entry.get("schema"))
                    }
                    SchemaSource::Resource(index) => self
                        .resources
                        .get(*index)
                        .and_then(|entry| entry.get("schema")),
                });
            if let Some(base) = base {
                admit_copy(
                    &mut self.materialized_nodes,
                    &mut self.materialized_bytes,
                    base,
                    file,
                    Some(expression.span),
                )?;
                Some(base.clone())
            } else {
                None
            }
        } else {
            None
        };
        let mut target = if suppress_children {
            format!("{id}:contract")
        } else {
            id
        };
        if let Some(body) = body {
            let mut paths = Vec::new();
            collect_default_override_paths(body, source, &mut Vec::new(), &mut paths);
            for path in paths {
                target = self.project_default_reference(&target, &path, file)?;
            }
        }
        let mut schema = Map::from_iter([("$ref".into(), Json::String(target))]);
        if let Some(body) = body {
            let narrowed =
                self.lower_expression(body, context, suppress_children, validate_children)?;
            let mut narrowed = Json::Object(narrowed);
            if let Some(base) = &base {
                self.normalize_conditional_default_overrides(
                    base,
                    &mut narrowed,
                    body,
                    source,
                    file,
                )?;
            }
            if !narrowed.as_object().is_some_and(Map::is_empty) {
                schema.insert("allOf".into(), Json::Array(vec![narrowed]));
            }
        }
        Ok(schema)
    }

    fn lower_tuple_tag(
        &mut self,
        name: Span,
        body: Option<&crate::native::Expression>,
        context: SourceContext<'_>,
        validate_children: bool,
    ) -> Result<Map<String, Json>, NativeError> {
        let Some(body) = body else {
            return Err(NativeError::new(
                NativeErrorKind::InvalidDefinition,
                "Tuple requires bracketed position constraints",
            )
            .at_path(context.file)
            .at_span(name));
        };
        let crate::native::ExpressionKind::Tuple(items) = &body.kind else {
            return Err(NativeError::new(
                NativeErrorKind::InvalidDefinition,
                "Tuple metadata must be bracketed position constraints",
            )
            .at_path(context.file)
            .at_span(body.span));
        };
        self.lower_tuple_positions(items, context, validate_children)
    }

    fn lower_tuple_positions(
        &mut self,
        items: &[crate::native::Expression],
        context: SourceContext<'_>,
        validate_children: bool,
    ) -> Result<Map<String, Json>, NativeError> {
        let mut schema = Map::new();
        schema.insert("type".into(), Json::String("array".into()));
        schema.insert("minItems".into(), Json::Number(Number::from(items.len())));
        schema.insert("maxItems".into(), Json::Number(Number::from(items.len())));
        schema.insert(
            "prefixItems".into(),
            Json::Array(
                items
                    .iter()
                    .map(|item| {
                        self.lower_expression(item, context, false, validate_children)
                            .map(Json::Object)
                    })
                    .collect::<Result<_, _>>()?,
            ),
        );
        Ok(schema)
    }
    fn lower_expression(
        &mut self,
        expression: &crate::native::Expression,
        context: SourceContext<'_>,
        suppress_root: bool,
        validate_children: bool,
    ) -> Result<Map<String, Json>, NativeError> {
        if self.lowering_depth >= crate::schema::profile::MAX_SCHEMA_DEPTH {
            return Err(NativeError::new(
                NativeErrorKind::LimitExceeded,
                "native definition lowering exceeds schema-depth bound",
            )
            .at_path(context.file)
            .at_span(expression.span));
        }
        self.lowering_depth += 1;
        let result =
            self.lower_expression_inner(expression, context, suppress_root, validate_children);
        self.lowering_depth -= 1;
        result
    }

    fn lower_literal(
        &mut self,
        node: &Node,
        source: &str,
        file: &Path,
    ) -> Result<Map<String, Json>, NativeError> {
        let mut schema = Map::new();
        match &node.value {
            Value::Temporal(value) => {
                let value = value.value(source).expect("admitted temporal source");
                admit_copy_cost(
                    &mut self.materialized_nodes,
                    &mut self.materialized_bytes,
                    4,
                    value.as_str().len() + 32,
                    file,
                    Some(node.span),
                )?;
                schema.insert("type".into(), Json::String("string".into()));
                schema.insert(
                    "x-daml-scalar".into(),
                    serde_json::json!({"kind":value.kind().as_str(),"const":value.as_str()}),
                );
            }
            Value::Sequence(items) => {
                admit_copy_cost(
                    &mut self.materialized_nodes,
                    &mut self.materialized_bytes,
                    6,
                    64,
                    file,
                    Some(node.span),
                )?;
                let children = items
                    .iter()
                    .map(|item| self.lower_literal(item, source, file).map(Json::Object))
                    .collect::<Result<Vec<_>, _>>()?;
                schema.insert("type".into(), Json::String("array".into()));
                schema.insert("prefixItems".into(), Json::Array(children));
                schema.insert("minItems".into(), Json::from(items.len()));
                schema.insert("maxItems".into(), Json::from(items.len()));
            }
            Value::Mapping(entries) => {
                let key_bytes = entries.iter().fold(0usize, |bytes, entry| {
                    bytes.saturating_add((entry.key.end - entry.key.start).saturating_mul(2))
                });
                admit_copy_cost(
                    &mut self.materialized_nodes,
                    &mut self.materialized_bytes,
                    entries.len().saturating_add(6),
                    key_bytes.saturating_add(64),
                    file,
                    Some(node.span),
                )?;
                let mut properties = Map::new();
                let mut required = Vec::with_capacity(entries.len());
                for entry in entries {
                    let key = &source[entry.key.start..entry.key.end];
                    properties.insert(
                        key.into(),
                        Json::Object(self.lower_literal(&entry.value, source, file)?),
                    );
                    required.push(Json::String(key.into()));
                }
                schema.insert("type".into(), Json::String("object".into()));
                schema.insert("properties".into(), Json::Object(properties));
                schema.insert("required".into(), Json::Array(required));
                schema.insert("additionalProperties".into(), Json::Bool(false));
            }
            _ => {
                schema.insert("const".into(), self.materialize_node(source, node, file)?);
                if matches!(node.value, Value::String(_)) {
                    schema.insert("x-daml-scalar".into(), serde_json::json!({"kind":"string"}));
                }
            }
        }
        Ok(schema)
    }

    fn lower_expression_inner(
        &mut self,
        expression: &crate::native::Expression,
        context: SourceContext<'_>,
        suppress_root: bool,
        validate_children: bool,
    ) -> Result<Map<String, Json>, NativeError> {
        let SourceContext { source, root, file } = context;
        use crate::native::{DefaultValue, ExpressionKind as K};
        let suppress_children =
            suppress_root || !matches!(&expression.default, DefaultValue::Inherited);
        let mut schema = Map::new();
        match &expression.kind {
            K::Literal(node) => {
                schema = self.lower_literal(node, source, file)?;
            }
            K::Regex(_) => {
                return Err(NativeError::new(
                    NativeErrorKind::InvalidDefinition,
                    "Regex is only valid as a String pattern facet",
                )
                .at_path(file)
                .at_span(expression.span));
            }
            K::Tagged { .. } => {
                schema =
                    self.lower_tagged(expression, context, suppress_children, validate_children)?;
            }
            K::Record(members) => {
                schema.insert("type".into(), Json::String("object".into()));
                let mut properties = Map::new();
                let mut required = Vec::new();
                for member in members {
                    let key = source.get(member.key.start..member.key.end).unwrap_or("");
                    let mut child = self.lower_expression(
                        &member.constraint,
                        context,
                        false,
                        validate_children,
                    )?;
                    if let Some(assignment) = first_unset_default_assignment(&member.constraint)
                        && !self.schema_proves_container(&child, file, assignment)?
                    {
                        return Err(NativeError::new(
                            NativeErrorKind::InvalidDefault,
                            "record-field unset requires a container constraint",
                        )
                        .at_path(file)
                        .at_span(assignment));
                    }
                    if member.optional {
                        child.insert("x-daml-native-optional".into(), Json::Bool(true));
                    }
                    properties.insert(key.into(), Json::Object(child));
                    if !member.optional {
                        required.push(Json::String(key.into()));
                    }
                }
                schema.insert("properties".into(), Json::Object(properties));
                if !required.is_empty() {
                    schema.insert("required".into(), Json::Array(required));
                }
            }
            K::Tuple(items) => {
                schema = self.lower_tuple_positions(items, context, validate_children)?;
            }
            K::Union(items) => {
                schema.insert(
                    "anyOf".into(),
                    Json::Array(
                        items
                            .iter()
                            .map(|item| {
                                self.lower_expression(
                                    item,
                                    context,
                                    suppress_children,
                                    validate_children,
                                )
                                .map(Json::Object)
                            })
                            .collect::<Result<_, _>>()?,
                    ),
                );
            }
            K::Intersection(items) => {
                schema.insert(
                    "allOf".into(),
                    Json::Array(self.lower_intersection(
                        items,
                        context,
                        suppress_children,
                        validate_children,
                    )?),
                );
            }
            K::Closed(inner) | K::Group(inner) => {
                schema =
                    self.lower_expression(inner, context, suppress_children, validate_children)?;
                if matches!(&expression.kind, K::Closed(_)) {
                    schema.insert("unevaluatedProperties".into(), Json::Bool(false));
                }
            }
        }

        if !suppress_root {
            match &expression.default {
                DefaultValue::Value(value) => {
                    schema.insert(
                        "default".into(),
                        self.materialize_node(source, value, file)?,
                    );
                    let tags = self.default_annotations(value, source, root, file)?;
                    insert_default_annotations(&mut schema, tags);
                }
                DefaultValue::Inherited | DefaultValue::Unset => {}
            }
        }
        Ok(schema)
    }

    fn validate_effective_defaults(
        &mut self,
        schema: &mut Json,
        file: &Path,
        span: Span,
    ) -> Result<(), NativeError> {
        let mut budget = WalkBudget {
            work: self.default_validation_steps,
            copied_nodes: self.materialized_nodes,
            copied_bytes: self.materialized_bytes,
            file,
            span,
        };
        let result = self.validate_effective_default_node(schema, file, span, &mut budget, 0);
        self.default_validation_steps = budget.work;
        self.materialized_nodes = budget.copied_nodes;
        self.materialized_bytes = budget.copied_bytes;
        result
    }
    fn schema_proves_container(
        &mut self,
        schema: &Map<String, Json>,
        file: &Path,
        span: Span,
    ) -> Result<bool, NativeError> {
        let mut budget = WalkBudget {
            work: self.default_validation_steps,
            copied_nodes: self.materialized_nodes,
            copied_bytes: self.materialized_bytes,
            file,
            span,
        };
        let result = self.schema_proves_container_inner(schema, &mut budget, 0);
        self.default_validation_steps = budget.work;
        self.materialized_nodes = budget.copied_nodes;
        self.materialized_bytes = budget.copied_bytes;
        result
    }

    fn schema_for_reference(&self, reference: &str) -> Option<&Json> {
        match self.schema_sources.get(reference) {
            Some(SchemaSource::Type(key)) => {
                self.types.get(key).and_then(|entry| entry.get("schema"))
            }
            Some(SchemaSource::Resource(index)) => self
                .resources
                .get(*index)
                .and_then(|entry| entry.get("schema")),
            None => None,
        }
    }

    fn schema_proves_container_inner(
        &self,
        schema: &Map<String, Json>,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<bool, NativeError> {
        budget.charge(depth)?;
        if matches!(schema.get("const"), Some(Json::Array(_) | Json::Object(_))) {
            return Ok(true);
        }
        if matches!(
            schema.get("type").and_then(Json::as_str),
            Some("object" | "array")
        ) {
            return Ok(true);
        }
        if let Some(types) = schema.get("type").and_then(Json::as_array)
            && !types.is_empty()
            && types
                .iter()
                .all(|value| matches!(value.as_str(), Some("object" | "array")))
        {
            return Ok(true);
        }
        if let Some(reference) = schema.get("$ref").and_then(Json::as_str) {
            let target = self.schema_for_reference(reference);
            if let Some(target) = target.and_then(Json::as_object)
                && self.schema_proves_container_inner(target, budget, depth + 1)?
            {
                return Ok(true);
            }
        }
        if let Some(branches) = schema.get("allOf").and_then(Json::as_array) {
            for branch in branches {
                if let Some(branch) = branch.as_object()
                    && self.schema_proves_container_inner(branch, budget, depth + 1)?
                {
                    return Ok(true);
                }
            }
        }
        let mut has_alternatives = false;
        for keyword in ["anyOf", "oneOf"] {
            if let Some(branches) = schema.get(keyword).and_then(Json::as_array) {
                if branches.is_empty() {
                    return Ok(false);
                }
                for branch in branches {
                    let Some(branch) = branch.as_object() else {
                        return Ok(false);
                    };
                    if !self.schema_proves_container_inner(branch, budget, depth + 1)? {
                        return Ok(false);
                    }
                }
                has_alternatives = true;
            }
        }
        Ok(has_alternatives)
    }

    fn validate_effective_default_node(
        &mut self,
        schema: &Json,
        file: &Path,
        span: Span,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        budget.charge(depth)?;
        let mut defaults = Vec::new();
        self.collect_default_candidates(schema, &mut HashSet::new(), &mut defaults, budget, depth)?;
        if let Some(schema) = schema.as_object() {
            for (value, annotations) in defaults {
                self.record_default_check(schema, value, annotations, file, span, budget)?;
            }
        }
        self.validate_union_branch_defaults(schema, file, span, budget, depth)?;
        let mut positions = HashSet::new();
        self.collect_schema_positions(schema, &mut HashSet::new(), &mut positions, budget, depth)?;
        for position in positions {
            if let Some(child) =
                self.effective_schema_child(schema, &position, &mut HashSet::new(), budget, depth)?
            {
                self.validate_effective_default_node(&child, file, span, budget, depth + 1)?;
            }
        }
        Ok(())
    }
    fn validate_union_branch_defaults(
        &mut self,
        schema: &Json,
        file: &Path,
        span: Span,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        let mut parts = Vec::new();
        self.collect_conjunctive_parts(schema, &mut HashSet::new(), &mut parts, budget, depth)?;
        for (index, part) in parts.iter().enumerate() {
            for branch in crate::schema::profile::children(part) {
                budget.charge(depth + 1)?;
                if branch.keyword() != "anyOf" {
                    continue;
                }
                let mut branch_parts = Vec::new();
                for (other_index, other) in parts.iter().enumerate() {
                    let Some(source_object) = other.as_object() else {
                        if other_index != index {
                            budget.charge_copy(other)?;
                            branch_parts.push(other.clone());
                        }
                        continue;
                    };
                    budget.charge_map_copy(source_object)?;
                    let mut object = source_object.clone();
                    object.remove("anyOf");
                    if !object.is_empty() {
                        branch_parts.push(Json::Object(object));
                    }
                }
                budget.charge_copy(branch.value)?;
                branch_parts.push(branch.value.clone());
                let branch_schema = Json::Object(Map::from_iter([(
                    "allOf".into(),
                    Json::Array(branch_parts),
                )]));
                self.validate_effective_default_node(
                    &branch_schema,
                    file,
                    span,
                    budget,
                    depth + 1,
                )?;
            }
        }
        Ok(())
    }

    fn collect_conjunctive_parts(
        &self,
        schema: &Json,
        visited: &mut HashSet<String>,
        parts: &mut Vec<Json>,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        budget.charge(depth)?;
        let Some(object) = schema.as_object() else {
            return Ok(());
        };
        if let Some(reference) = object.get("$ref").and_then(Json::as_str)
            && visited.insert(reference.to_owned())
        {
            let target = self.schema_for_reference(reference);
            if let Some(target) = target {
                self.collect_conjunctive_parts(target, visited, parts, budget, depth + 1)?;
            }
        }
        for child in crate::schema::profile::children(schema) {
            if child.keyword() == "allOf" {
                self.collect_conjunctive_parts(child.value, visited, parts, budget, depth + 1)?;
            }
        }
        budget.charge_map_copy(object)?;
        let mut local = object.clone();
        local.remove("$id");
        local.remove("$ref");
        local.remove("allOf");
        if !local.is_empty() {
            parts.push(Json::Object(local));
        }
        Ok(())
    }

    fn collect_default_candidates(
        &self,
        schema: &Json,
        visited: &mut HashSet<String>,
        found: &mut Vec<(Json, Vec<Json>)>,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        budget.charge(depth)?;
        if let Some(object) = schema.as_object() {
            if let Some(value) = object.get("default") {
                let mut annotations = Vec::new();
                budget.charge_copy(value)?;
                for annotation in schema_default_annotations(object) {
                    budget.charge_copy(annotation)?;
                    annotations.push(annotation.clone());
                }
                found.push((value.clone(), annotations));
            }
            if let Some(reference) = object.get("$ref").and_then(Json::as_str)
                && visited.insert(reference.to_owned())
            {
                let target = self.schema_for_reference(reference);
                if let Some(target) = target {
                    self.collect_default_candidates(target, visited, found, budget, depth + 1)?;
                }
            }
            for child in crate::schema::profile::children(schema) {
                if matches!(child.keyword(), "allOf" | "anyOf" | "oneOf") {
                    self.collect_default_candidates(
                        child.value,
                        visited,
                        found,
                        budget,
                        depth + 1,
                    )?;
                }
            }
        }
        Ok(())
    }
    fn collect_schema_positions(
        &self,
        schema: &Json,
        visited: &mut HashSet<String>,
        positions: &mut HashSet<SchemaPosition>,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        budget.charge(depth)?;
        for child in crate::schema::profile::children(schema) {
            if !child.applies || child.unsupported_defaults {
                continue;
            }
            let keyword = child.keyword();
            match keyword {
                "allOf" | "anyOf" | "oneOf" => {
                    self.collect_schema_positions(
                        child.value,
                        visited,
                        positions,
                        budget,
                        depth + 1,
                    )?;
                }
                "properties" => {
                    positions.insert(SchemaPosition {
                        keyword: "property".to_owned(),
                        key: child.map_key().map(str::to_owned),
                        index: None,
                    });
                }
                "additionalProperties" => {
                    positions.insert(SchemaPosition {
                        keyword: "property".to_owned(),
                        key: None,
                        index: None,
                    });
                }
                "prefixItems" => {
                    positions.insert(SchemaPosition {
                        keyword: "item".to_owned(),
                        key: None,
                        index: child.array_index(),
                    });
                }
                "items" => {
                    positions.insert(SchemaPosition {
                        keyword: "item".to_owned(),
                        key: None,
                        index: None,
                    });
                }
                _ if child.map_key().is_some() || child.array_index().is_some() => {
                    positions.insert(SchemaPosition {
                        keyword: keyword.to_owned(),
                        key: child.map_key().map(str::to_owned),
                        index: child.array_index(),
                    });
                }
                _ => {}
            }
        }
        if let Some(reference) = schema.get("$ref").and_then(Json::as_str)
            && visited.insert(reference.to_owned())
        {
            let target = self.schema_for_reference(reference);
            if let Some(target) = target {
                self.collect_schema_positions(target, visited, positions, budget, depth + 1)?;
            }
        }
        Ok(())
    }

    fn effective_schema_child(
        &self,
        schema: &Json,
        position: &SchemaPosition,
        visited: &mut HashSet<String>,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<Option<Json>, NativeError> {
        budget.charge(depth)?;
        let mut conjuncts = Vec::new();
        let selected = match position.keyword.as_str() {
            "property" => position
                .key
                .as_deref()
                .and_then(|key| crate::schema::profile::property(schema, key))
                .or_else(|| {
                    position
                        .key
                        .is_none()
                        .then(|| schema.get("additionalProperties"))
                        .flatten()
                }),
            "item" => position
                .index
                .and_then(|index| crate::schema::profile::item(schema, index))
                .or_else(|| {
                    position
                        .index
                        .is_none()
                        .then(|| schema.get("items"))
                        .flatten()
                }),
            _ => crate::schema::profile::children(schema)
                .find(|child| {
                    child.keyword() == position.keyword
                        && child.map_key() == position.key.as_deref()
                        && child.array_index() == position.index
                })
                .map(|child| child.value),
        };
        if let Some(child) = selected {
            budget.charge_copy(child)?;
            conjuncts.push(child.clone());
        }
        if let Some(reference) = schema.get("$ref").and_then(Json::as_str)
            && visited.insert(reference.to_owned())
        {
            let target = self.schema_for_reference(reference);
            if let Some(target) = target
                && let Some(child) =
                    self.effective_schema_child(target, position, visited, budget, depth + 1)?
            {
                conjuncts.push(child);
            }
        }
        if let Some(branches) = schema.get("allOf").and_then(Json::as_array) {
            for branch in branches {
                if let Some(child) =
                    self.effective_schema_child(branch, position, visited, budget, depth + 1)?
                {
                    conjuncts.push(child);
                }
            }
        }
        let mut union = Vec::new();
        if let Some(branches) = schema.get("anyOf").and_then(Json::as_array) {
            for branch in branches {
                union.push(
                    self.effective_schema_child(
                        branch,
                        position,
                        &mut visited.clone(),
                        budget,
                        depth + 1,
                    )?
                    .unwrap_or(Json::Bool(true)),
                );
            }
        }
        if !union.is_empty() {
            conjuncts.push(Json::Object(Map::from_iter([(
                "anyOf".into(),
                Json::Array(union),
            )])));
        }
        Ok(match conjuncts.len() {
            0 => None,
            1 => conjuncts.pop(),
            _ => Some(Json::Object(Map::from_iter([(
                "allOf".into(),
                Json::Array(conjuncts),
            )]))),
        })
    }

    fn collect_unconditional_defaults(
        &self,
        schema: &Json,
        visited: &mut HashSet<String>,
        found: &mut Vec<(Json, Vec<Json>)>,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        budget.charge(depth)?;
        if let Some(object) = schema.as_object() {
            if let Some(value) = object.get("default") {
                let mut annotations = Vec::new();
                budget.charge_copy(value)?;
                for annotation in schema_default_annotations(object) {
                    budget.charge_copy(annotation)?;
                    annotations.push(annotation.clone());
                }
                found.push((value.clone(), annotations));
            }
            if let Some(reference) = object.get("$ref").and_then(Json::as_str)
                && visited.insert(reference.to_owned())
            {
                let target = self.schema_for_reference(reference);
                if let Some(target) = target {
                    self.collect_unconditional_defaults(target, visited, found, budget, depth + 1)?;
                }
            }
            if let Some(all_of) = object.get("allOf").and_then(Json::as_array) {
                for branch in all_of {
                    self.collect_unconditional_defaults(branch, visited, found, budget, depth + 1)?;
                }
            }
        }
        Ok(())
    }
    fn normalize_conditional_default_overrides(
        &mut self,
        base: &Json,
        narrowed: &mut Json,
        expression: &crate::native::Expression,
        source: &str,
        file: &Path,
    ) -> Result<(), NativeError> {
        self.normalize_conditional_default_overrides_from_schemas(
            std::slice::from_ref(base),
            narrowed,
            expression,
            source,
            file,
        )
    }

    fn normalize_conditional_default_overrides_from_schemas(
        &mut self,
        base_schemas: &[Json],
        narrowed: &mut Json,
        expression: &crate::native::Expression,
        source: &str,
        file: &Path,
    ) -> Result<(), NativeError> {
        let mut budget = WalkBudget {
            work: self.default_validation_steps,
            copied_nodes: self.materialized_nodes,
            copied_bytes: self.materialized_bytes,
            file,
            span: expression.span,
        };
        let result = self.normalize_conditional_default_tree(
            InheritedDefaults {
                schemas: base_schemas,
                presets: &[],
                root_replaced: false,
            },
            narrowed,
            expression,
            source,
            &mut budget,
            0,
        );
        self.default_validation_steps = budget.work;
        self.materialized_nodes = budget.copied_nodes;
        self.materialized_bytes = budget.copied_bytes;
        result
    }
    fn normalize_conditional_default_tree(
        &mut self,
        inherited: InheritedDefaults<'_>,
        local_schema: &mut Json,
        expression: &crate::native::Expression,
        source: &str,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        let InheritedDefaults {
            schemas: base_schemas,
            presets: ancestor_presets,
            root_replaced: inherited_root_replacement,
        } = inherited;
        let file = budget.file;
        budget.charge(depth)?;
        let root_replaced = inherited_root_replacement
            || !matches!(
                expression.effective_default(),
                crate::native::DefaultValue::Inherited
            );
        use crate::native::ExpressionKind as K;
        match &expression.kind {
            K::Record(members) => {
                for member in members {
                    let key = source.get(member.key.start..member.key.end).unwrap_or("");
                    let mut inherited = Vec::new();
                    let mut child_presets = Vec::new();
                    for preset in ancestor_presets {
                        if let Some(child) = extract_preset_component(
                            preset,
                            &DefaultPathStep::Property(key.to_owned()),
                            budget,
                        )? {
                            child_presets.push(child);
                        }
                    }
                    for base in base_schemas {
                        self.collect_property_schemas(
                            base,
                            key,
                            &mut HashSet::new(),
                            &mut inherited,
                            budget,
                            depth + 1,
                        )?;
                        let mut defaults = Vec::new();
                        self.collect_unconditional_defaults(
                            base,
                            &mut HashSet::new(),
                            &mut defaults,
                            budget,
                            depth + 1,
                        )?;
                        for (value, annotations) in defaults {
                            if let Some(child) = extract_preset_component(
                                &PresetFragment { value, annotations },
                                &DefaultPathStep::Property(key.to_owned()),
                                budget,
                            )? {
                                child_presets.push(child);
                            }
                        }
                    }
                    let Some(local) = local_schema
                        .get_mut("properties")
                        .and_then(Json::as_object_mut)
                        .and_then(|properties| properties.get_mut(key))
                    else {
                        continue;
                    };
                    self.normalize_conditional_default_tree(
                        InheritedDefaults {
                            schemas: &inherited,
                            presets: &child_presets,
                            root_replaced: false,
                        },
                        local,
                        &member.constraint,
                        source,
                        budget,
                        depth + 1,
                    )?;
                }
            }
            K::Union(items) => {
                let has_overrides = items.iter().any(|expression| {
                    let mut paths = Vec::new();
                    collect_unconditional_default_override_paths(
                        expression,
                        source,
                        &mut Vec::new(),
                        &mut paths,
                    );
                    !paths.is_empty()
                });
                if !has_overrides || (base_schemas.is_empty() && ancestor_presets.is_empty()) {
                    return Ok(());
                }
                let Some(branches) = local_schema.get_mut("anyOf").and_then(Json::as_array_mut)
                else {
                    return Ok(());
                };
                let enclosing_replaced = root_replaced;
                let mut union_presets = if enclosing_replaced {
                    Vec::new()
                } else {
                    for preset in ancestor_presets {
                        budget.charge_preset_copy(preset)?;
                    }
                    ancestor_presets.to_vec()
                };
                if !enclosing_replaced {
                    for schema in base_schemas {
                        let mut defaults = Vec::new();
                        self.collect_unconditional_defaults(
                            schema,
                            &mut HashSet::new(),
                            &mut defaults,
                            budget,
                            depth + 1,
                        )?;
                        union_presets.extend(
                            defaults
                                .into_iter()
                                .map(|(value, annotations)| PresetFragment { value, annotations }),
                        );
                    }
                }
                for (expression, branch) in items.iter().zip(branches) {
                    let mut paths = Vec::new();
                    if enclosing_replaced {
                        paths.push(Vec::new());
                    }
                    collect_unconditional_default_override_paths(
                        expression,
                        source,
                        &mut Vec::new(),
                        &mut paths,
                    );
                    for schema in base_schemas {
                        budget.charge_copy(schema)?;
                    }
                    let mut inherited = base_schemas.to_vec();
                    for schema in &mut inherited {
                        for path in &paths {
                            let mut projection_budget = Some(&mut *budget);
                            self.project_schema_defaults(
                                schema,
                                path,
                                file,
                                &mut HashSet::new(),
                                &mut projection_budget,
                            )?;
                        }
                    }
                    for schema in &inherited {
                        budget.charge_copy(schema)?;
                    }
                    let branch_bases = inherited.clone();
                    let root_replaced = paths.iter().any(Vec::is_empty);
                    for preset in &union_presets {
                        budget.charge_preset_copy(preset)?;
                    }
                    let mut branch_presets = if root_replaced {
                        Vec::new()
                    } else {
                        union_presets.clone()
                    };
                    let object = branch.as_object_mut().expect("native union branch");
                    let all_of = object
                        .entry("allOf")
                        .or_insert_with(|| Json::Array(Vec::new()))
                        .as_array_mut()
                        .expect("native allOf is an array");
                    all_of.extend(inherited);
                    for preset in &mut branch_presets {
                        for path in &paths {
                            remove_preset_component(preset, path);
                        }
                        let mut default_schema = Map::new();
                        budget.charge_copy(&preset.value)?;
                        default_schema.insert("default".into(), preset.value.clone());
                        if !preset.annotations.is_empty() {
                            for annotation in &preset.annotations {
                                budget.charge_copy(annotation)?;
                            }
                            insert_default_annotations(
                                &mut default_schema,
                                preset.annotations.clone(),
                            );
                        }
                        all_of.push(Json::Object(default_schema));
                    }
                    self.normalize_conditional_default_tree(
                        InheritedDefaults {
                            schemas: &branch_bases,
                            presets: &branch_presets,
                            root_replaced: false,
                        },
                        branch,
                        expression,
                        source,
                        budget,
                        depth + 1,
                    )?;
                }
            }
            K::Tagged { name, body } => {
                if let Some(body) = body {
                    let narrowed = if source.get(name.start..name.end) == Some("Tuple") {
                        Some(local_schema)
                    } else {
                        local_schema
                            .get_mut("allOf")
                            .and_then(Json::as_array_mut)
                            .and_then(|all_of| all_of.first_mut())
                    };
                    if let Some(narrowed) = narrowed {
                        self.normalize_conditional_default_tree(
                            InheritedDefaults {
                                schemas: base_schemas,
                                presets: ancestor_presets,
                                root_replaced,
                            },
                            narrowed,
                            body,
                            source,
                            budget,
                            depth + 1,
                        )?;
                    }
                }
            }
            K::Closed(inner) | K::Group(inner) => {
                self.normalize_conditional_default_tree(
                    InheritedDefaults {
                        schemas: base_schemas,
                        presets: ancestor_presets,
                        root_replaced,
                    },
                    local_schema,
                    inner,
                    source,
                    budget,
                    depth + 1,
                )?;
            }
            K::Tuple(items) => {
                for (index, item) in items.iter().enumerate() {
                    let mut inherited = Vec::new();
                    for base in base_schemas {
                        self.collect_item_schemas(
                            base,
                            index,
                            &mut HashSet::new(),
                            &mut inherited,
                            budget,
                            depth + 1,
                        )?;
                    }
                    let Some(local) = local_schema
                        .get_mut("prefixItems")
                        .and_then(Json::as_array_mut)
                        .and_then(|prefix| prefix.get_mut(index))
                    else {
                        continue;
                    };
                    self.normalize_conditional_default_tree(
                        InheritedDefaults {
                            schemas: &inherited,
                            presets: &[],
                            root_replaced: false,
                        },
                        local,
                        item,
                        source,
                        budget,
                        depth + 1,
                    )?;
                }
            }
            K::Literal(_) | K::Regex(_) => {}
            K::Intersection(items) => {
                if let Some(branches) = local_schema.get_mut("allOf").and_then(Json::as_array_mut) {
                    for (item, branch) in items.iter().zip(branches) {
                        self.normalize_conditional_default_tree(
                            InheritedDefaults {
                                schemas: base_schemas,
                                presets: ancestor_presets,
                                root_replaced,
                            },
                            branch,
                            item,
                            source,
                            budget,
                            depth + 1,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    fn collect_item_schemas(
        &self,
        schema: &Json,
        index: usize,
        visited: &mut HashSet<String>,
        found: &mut Vec<Json>,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        let position = SchemaPosition {
            keyword: "item".into(),
            key: None,
            index: Some(index),
        };
        if let Some(item) =
            self.effective_schema_child(schema, &position, visited, budget, depth)?
        {
            found.push(item);
        }
        Ok(())
    }

    fn collect_property_schemas(
        &self,
        schema: &Json,
        key: &str,
        visited: &mut HashSet<String>,
        found: &mut Vec<Json>,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        budget.charge(depth)?;
        if let Some(object) = schema.as_object() {
            if let Some(property) = crate::schema::profile::property(schema, key) {
                budget.charge_copy(property)?;
                found.push(property.clone());
            }
            if let Some(reference) = object.get("$ref").and_then(Json::as_str)
                && visited.insert(reference.to_owned())
            {
                let target = self.schema_for_reference(reference);
                if let Some(target) = target {
                    self.collect_property_schemas(target, key, visited, found, budget, depth + 1)?;
                }
            }
            if let Some(all_of) = object.get("allOf").and_then(Json::as_array) {
                for branch in all_of {
                    self.collect_property_schemas(branch, key, visited, found, budget, depth + 1)?;
                }
            }
        }
        Ok(())
    }

    fn default_annotations(
        &mut self,
        node: &Node,
        source: &str,
        root: &Path,
        file: &Path,
    ) -> Result<Vec<Json>, NativeError> {
        fn walk(
            compiler: &mut Compiler<'_>,
            node: &Node,
            context: SourceContext<'_>,

            path: &str,
            out: &mut Vec<Json>,
        ) -> Result<(), NativeError> {
            let SourceContext { source, root, file } = context;
            if let Some(span) = node.tag {
                let tag = source.get(span.start..span.end).unwrap_or("");
                let (binding, _) = compiler.load_tag(tag, Some(span), root).map_err(|error| {
                    if error.path.is_none() {
                        error.at_path(file)
                    } else {
                        error
                    }
                })?;
                admit_copy_cost(
                    &mut compiler.materialized_nodes,
                    &mut compiler.materialized_bytes,
                    4,
                    "path".len()
                        + "tag".len()
                        + "binding".len()
                        + path.len()
                        + tag.len()
                        + binding.len(),
                    file,
                    Some(span),
                )?;
                out.push(Json::Object(Map::from_iter([
                    ("path".into(), Json::String(path.to_owned())),
                    ("tag".into(), Json::String(tag.to_owned())),
                    ("binding".into(), Json::String(binding)),
                ])));
            }
            if let Value::Temporal(value) = &node.value {
                let value = value.value(source).expect("admitted temporal source");
                let annotation = serde_json::json!({"path":path,"kind":value.kind().as_str()});
                admit_copy(
                    &mut compiler.materialized_nodes,
                    &mut compiler.materialized_bytes,
                    &annotation,
                    file,
                    Some(node.span),
                )?;
                out.push(annotation);
            }
            match &node.value {
                Value::Sequence(items) => {
                    for (index, item) in items.iter().enumerate() {
                        walk(compiler, item, context, &format!("{path}/{index}"), out)?;
                    }
                }
                Value::Mapping(entries) => {
                    for entry in entries {
                        let key = source.get(entry.key.start..entry.key.end).unwrap_or("");
                        walk(
                            compiler,
                            &entry.value,
                            context,
                            &crate::pointer::join(path, key),
                            out,
                        )?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        let mut annotations = Vec::new();
        walk(
            self,
            node,
            SourceContext { source, root, file },
            "",
            &mut annotations,
        )?;
        Ok(annotations)
    }
    fn record_default_check(
        &mut self,
        schema: &Map<String, Json>,
        value: Json,
        annotations: Vec<Json>,
        file: &Path,
        span: Span,
        budget: &mut WalkBudget<'_>,
    ) -> Result<(), NativeError> {
        budget.charge_map_copy(schema)?;
        let mut value_schema = Json::Object(schema.clone());
        let object = value_schema.as_object_mut().expect("constructed object");
        object.remove("default");
        object.remove("x-daml-default-tags");
        object.remove("x-daml-default-temporals");
        let ordinal = self.checks.len();
        let name = format!("native_default_check_{ordinal}");
        let id = format!("urn:daml:native:default-check:{ordinal}");
        value_schema
            .as_object_mut()
            .expect("constructed schema")
            .insert("$id".into(), Json::String(id));
        self.types.insert(
            name.clone(),
            Json::Object(Map::from_iter([("schema".into(), value_schema)])),
        );
        self.checks.push(DefaultCheck {
            schema_type: name,
            value,
            annotations,
            path: file.to_path_buf(),
            span,
        });
        Ok(())
    }
}

fn visit_conjunction_leaves_mut<E>(
    expression: &crate::native::Expression,
    schema: &mut Json,
    visit: &mut impl FnMut(&crate::native::Expression, &mut Json) -> Result<(), E>,
) -> Result<(), E> {
    use crate::native::{DefaultValue, ExpressionKind as K};
    let mut operands = vec![(expression, schema)];
    while let Some((operand, schema)) = operands.pop() {
        match &operand.kind {
            K::Group(inner) | K::Closed(inner)
                if matches!(operand.effective_default(), DefaultValue::Inherited) =>
            {
                operands.push((inner, schema));
            }
            K::Intersection(children)
                if matches!(operand.effective_default(), DefaultValue::Inherited) =>
            {
                let Some(Json::Array(child_schemas)) = schema.get_mut("allOf") else {
                    continue;
                };
                for (child, child_schema) in children.iter().zip(child_schemas) {
                    operands.push((child, child_schema));
                }
            }
            _ => visit(operand, schema)?,
        }
    }
    Ok(())
}

fn supports_default_override_projection(expression: &crate::native::Expression) -> bool {
    use crate::native::ExpressionKind as K;
    match &expression.kind {
        K::Record(_) | K::Tuple(_) => true,
        K::Tagged { body, .. } => body
            .as_deref()
            .is_some_and(supports_default_override_projection),
        K::Closed(inner) | K::Group(inner) => supports_default_override_projection(inner),
        K::Literal(_) | K::Regex(_) | K::Union(_) | K::Intersection(_) => false,
    }
}

fn collect_default_override_paths(
    expression: &crate::native::Expression,
    source: &str,
    prefix: &mut DefaultPath,
    out: &mut Vec<DefaultPath>,
) {
    use crate::native::ExpressionKind as K;
    if !matches!(
        expression.effective_default(),
        crate::native::DefaultValue::Inherited
    ) {
        out.push(prefix.clone());
    }
    match &expression.kind {
        K::Record(members) => {
            for member in members {
                let key = source.get(member.key.start..member.key.end).unwrap_or("");
                prefix.push(DefaultPathStep::Property(key.to_owned()));
                collect_default_override_paths(&member.constraint, source, prefix, out);
                prefix.pop();
            }
        }
        K::Tagged { body, .. } => {
            if let Some(body) = body {
                collect_default_override_paths(body, source, prefix, out);
            }
        }
        K::Union(items) | K::Intersection(items) => {
            for item in items {
                collect_default_override_paths(item, source, prefix, out);
            }
        }
        K::Closed(inner) | K::Group(inner) => {
            collect_default_override_paths(inner, source, prefix, out);
        }
        K::Tuple(items) => {
            for (index, item) in items.iter().enumerate() {
                prefix.push(DefaultPathStep::Item(index));
                collect_default_override_paths(item, source, prefix, out);
                prefix.pop();
            }
        }
        K::Literal(_) | K::Regex(_) => {}
    }
}

fn collect_unconditional_default_override_paths(
    expression: &crate::native::Expression,
    source: &str,
    prefix: &mut DefaultPath,
    out: &mut Vec<DefaultPath>,
) {
    use crate::native::ExpressionKind as K;
    if !matches!(
        expression.effective_default(),
        crate::native::DefaultValue::Inherited
    ) {
        out.push(prefix.clone());
    }
    match &expression.kind {
        K::Record(members) => {
            for member in members {
                let key = source.get(member.key.start..member.key.end).unwrap_or("");
                prefix.push(DefaultPathStep::Property(key.to_owned()));
                collect_unconditional_default_override_paths(
                    &member.constraint,
                    source,
                    prefix,
                    out,
                );
                prefix.pop();
            }
        }
        K::Tagged { body, .. } => {
            if let Some(body) = body {
                collect_unconditional_default_override_paths(body, source, prefix, out);
            }
        }
        K::Union(_) => {}
        K::Intersection(items) => {
            for item in items {
                collect_unconditional_default_override_paths(item, source, prefix, out);
            }
        }
        K::Closed(inner) | K::Group(inner) => {
            collect_unconditional_default_override_paths(inner, source, prefix, out);
        }
        K::Tuple(items) => {
            for (index, item) in items.iter().enumerate() {
                prefix.push(DefaultPathStep::Item(index));
                collect_unconditional_default_override_paths(item, source, prefix, out);
                prefix.pop();
            }
        }
        K::Literal(_) | K::Regex(_) => {}
    }
}

fn default_path_pointer(path: &[DefaultPathStep]) -> String {
    path.iter().fold(String::new(), |pointer, step| match step {
        DefaultPathStep::Property(key) => crate::pointer::join(&pointer, key),
        DefaultPathStep::Item(index) => crate::pointer::join(&pointer, &index.to_string()),
    })
}

fn json_cost(value: &Json) -> (usize, usize) {
    let mut nodes = 1usize;
    let mut bytes = 0usize;
    match value {
        Json::String(value) => bytes = value.len(),
        Json::Number(value) => bytes = value.as_str().len(),
        Json::Array(values) => {
            for child in values {
                let (child_nodes, child_bytes) = json_cost(child);
                nodes = nodes.saturating_add(child_nodes);
                bytes = bytes.saturating_add(child_bytes);
            }
        }
        Json::Object(values) => {
            for (key, child) in values {
                bytes = bytes.saturating_add(key.len());
                let (child_nodes, child_bytes) = json_cost(child);
                nodes = nodes.saturating_add(child_nodes);
                bytes = bytes.saturating_add(child_bytes);
            }
        }
        Json::Null | Json::Bool(_) => {}
    }
    (nodes, bytes)
}

fn json_map_cost(value: &Map<String, Json>) -> (usize, usize) {
    let mut nodes = 1usize;
    let mut bytes = 0usize;
    for (key, child) in value {
        bytes = bytes.saturating_add(key.len());
        let (child_nodes, child_bytes) = json_cost(child);
        nodes = nodes.saturating_add(child_nodes);
        bytes = bytes.saturating_add(child_bytes);
    }
    (nodes, bytes)
}

fn admit_copy(
    copied_nodes: &mut usize,
    copied_bytes: &mut usize,
    value: &Json,
    file: &Path,
    span: Option<Span>,
) -> Result<(), NativeError> {
    let (nodes, bytes) = json_cost(value);
    admit_copy_cost(copied_nodes, copied_bytes, nodes, bytes, file, span)
}

fn admit_copy_cost(
    copied_nodes: &mut usize,
    copied_bytes: &mut usize,
    nodes: usize,
    bytes: usize,
    file: &Path,
    span: Option<Span>,
) -> Result<(), NativeError> {
    if nodes > crate::schema::profile::MAX_COMPILE_NODES.saturating_sub(*copied_nodes)
        || bytes > crate::schema::profile::MAX_RAW_WORK_BYTES.saturating_sub(*copied_bytes)
    {
        let error = NativeError::new(
            NativeErrorKind::LimitExceeded,
            "native compiler materialization exceeds node/byte budget",
        )
        .at_path(file);
        return Err(match span {
            Some(span) => error.at_span(span),
            None => error,
        });
    }
    *copied_nodes += nodes;
    *copied_bytes += bytes;
    Ok(())
}

fn schema_default_annotations(schema: &Map<String, Json>) -> impl Iterator<Item = &Json> {
    ["x-daml-default-tags", "x-daml-default-temporals"]
        .into_iter()
        .filter_map(|keyword| schema.get(keyword).and_then(Json::as_array))
        .flatten()
}
fn insert_default_annotations(schema: &mut Map<String, Json>, annotations: Vec<Json>) {
    let (temporals, tags): (Vec<_>, Vec<_>) = annotations
        .into_iter()
        .partition(|annotation| annotation.get("kind").is_some());
    if !tags.is_empty() {
        schema.insert("x-daml-default-tags".into(), Json::Array(tags));
    }
    if !temporals.is_empty() {
        schema.insert("x-daml-default-temporals".into(), Json::Array(temporals));
    }
}

fn extract_preset_component(
    preset: &PresetFragment,
    step: &DefaultPathStep,
    budget: &mut WalkBudget<'_>,
) -> Result<Option<PresetFragment>, NativeError> {
    let (value, parent) = match step {
        DefaultPathStep::Property(key) => (
            preset.value.as_object().and_then(|object| object.get(key)),
            crate::pointer::join("", key),
        ),
        DefaultPathStep::Item(index) => (
            preset.value.as_array().and_then(|items| items.get(*index)),
            crate::pointer::join("", &index.to_string()),
        ),
    };
    let Some(value) = value else {
        return Ok(None);
    };
    budget.charge_copy(value)?;
    let descendant = format!("{parent}/");
    let mut annotations = Vec::new();
    for annotation in &preset.annotations {
        let Some(path) = annotation.get("path").and_then(Json::as_str) else {
            continue;
        };
        let rebased = if path == parent {
            Some(String::new())
        } else {
            path.strip_prefix(&descendant)
                .map(|path| format!("/{path}"))
        };
        let Some(rebased) = rebased else {
            continue;
        };
        budget.charge_copy(annotation)?;
        let mut annotation = annotation.clone();
        annotation
            .as_object_mut()
            .expect("admitted annotation is an object")
            .insert("path".into(), Json::String(rebased));
        annotations.push(annotation);
    }
    Ok(Some(PresetFragment {
        value: value.clone(),
        annotations,
    }))
}

fn remove_preset_component(preset: &mut PresetFragment, path: &[DefaultPathStep]) {
    remove_default_component(&mut preset.value, path);
    let pointer = default_path_pointer(path);
    let prefix = format!("{pointer}/");
    preset.annotations.retain(|annotation| {
        annotation
            .get("path")
            .and_then(Json::as_str)
            .is_none_or(|path| path != pointer && !path.starts_with(&prefix))
    });
}

fn remove_default_component(value: &mut Json, path: &[DefaultPathStep]) -> bool {
    let Some((step, rest)) = path.split_first() else {
        return false;
    };
    match step {
        DefaultPathStep::Property(key) => {
            let Some(object) = value.as_object_mut() else {
                return false;
            };
            if rest.is_empty() {
                object.remove(key).is_some()
            } else {
                object
                    .get_mut(key)
                    .is_some_and(|child| remove_default_component(child, rest))
            }
        }
        DefaultPathStep::Item(index) => {
            let Some(items) = value.as_array_mut() else {
                return false;
            };
            items
                .get_mut(*index)
                .is_some_and(|child| remove_default_component(child, rest))
        }
    }
}

fn count_facet_bound(
    name: &str,
    key: &str,
    expression: &crate::native::Expression,
    source: &str,
    file: &Path,
) -> Result<u64, NativeError> {
    use crate::native::ExpressionKind as K;
    let error = |span| {
        NativeError::new(
            NativeErrorKind::InvalidDefinition,
            format!("{name} facet {key:?} requires a non-negative integer"),
        )
        .at_path(file)
        .at_span(span)
    };
    let K::Literal(value) = &expression.kind else {
        return Err(error(expression.span));
    };
    let Value::Number(span) = &value.value else {
        return Err(error(value.span));
    };
    source
        .get(span.start..span.end)
        .and_then(|raw| raw.parse().ok())
        .ok_or_else(|| error(value.span))
}

fn is_positive_number_literal(raw: &str) -> bool {
    let significand_end = raw.find(['e', 'E']).unwrap_or(raw.len());
    !raw.starts_with('-')
        && raw[..significand_end]
            .bytes()
            .any(|byte| matches!(byte, b'1'..=b'9'))
}

fn builtin(name: &str) -> Option<Map<String, Json>> {
    let mut schema = Map::new();
    match name {
        "String" => {
            schema.insert("type".into(), Json::String("string".into()));
            schema.insert("x-daml-scalar".into(), serde_json::json!({"kind":"string"}));
        }
        "Date" | "Time" | "DateTime" => {
            let kind = crate::TemporalKind::from_constructor(name).expect("matched intrinsic");
            schema.insert("type".into(), Json::String("string".into()));
            schema.insert(
                "x-daml-scalar".into(),
                serde_json::json!({"kind":kind.as_str()}),
            );
        }
        "Bool" => {
            schema.insert("type".into(), Json::String("boolean".into()));
        }
        "Int" => {
            schema.insert("type".into(), Json::String("integer".into()));
        }
        "Number" => {
            schema.insert("type".into(), Json::String("number".into()));
        }
        "Object" => {
            schema.insert("type".into(), Json::String("object".into()));
        }
        "List" => {
            schema.insert("type".into(), Json::String("array".into()));
        }
        "Tuple" => {
            schema.insert("type".into(), Json::String("array".into()));
        }
        "Map" => {
            schema.insert("type".into(), Json::String("object".into()));
        }
        "Any" => return Some(schema),
        "Never" => return None,
        _ => return None,
    }
    Some(schema)
}

fn node_json_cost(source: &str, node: &Node) -> (usize, usize) {
    let (mut nodes, mut bytes) = (1usize, 0usize);
    match &node.value {
        Value::String(StringValue::Decoded(text)) => bytes = text.len(),
        Value::String(StringValue::Source(span)) | Value::Number(span) => {
            bytes = source.get(span.start..span.end).map_or(0, str::len);
        }
        Value::Temporal(value) => {
            bytes = value
                .value(source)
                .expect("admitted temporal source")
                .as_str()
                .len()
        }
        Value::Sequence(items) => {
            for item in items {
                let (child_nodes, child_bytes) = node_json_cost(source, item);
                nodes = nodes.saturating_add(child_nodes);
                bytes = bytes.saturating_add(child_bytes);
            }
        }
        Value::Mapping(entries) => {
            for entry in entries {
                bytes = bytes.saturating_add(
                    source
                        .get(entry.key.start..entry.key.end)
                        .map_or(0, str::len),
                );
                let (child_nodes, child_bytes) = node_json_cost(source, &entry.value);
                nodes = nodes.saturating_add(child_nodes);
                bytes = bytes.saturating_add(child_bytes);
            }
        }
        Value::Null | Value::Bool(_) => {}
    }
    (nodes, bytes)
}

fn has_default_in_expression_tree(expression: &crate::native::Expression) -> bool {
    use crate::native::{DefaultValue, ExpressionKind as K};
    if !matches!(&expression.default, DefaultValue::Inherited) {
        return true;
    }
    match &expression.kind {
        K::Group(inner) | K::Closed(inner) => has_default_in_expression_tree(inner),
        K::Union(items) | K::Intersection(items) => {
            items.iter().any(has_default_in_expression_tree)
        }
        K::Literal(_) | K::Regex(_) | K::Tagged { .. } | K::Record(_) | K::Tuple(_) => false,
    }
}

fn first_unset_default_assignment(expression: &crate::native::Expression) -> Option<Span> {
    use crate::native::{DefaultValue, ExpressionKind as K};
    if matches!(expression.default, DefaultValue::Unset) {
        return Some(expression.assignment.unwrap_or(expression.span));
    }
    match &expression.kind {
        K::Group(inner) | K::Closed(inner) => first_unset_default_assignment(inner),
        K::Union(items) | K::Intersection(items) => {
            items.iter().find_map(first_unset_default_assignment)
        }
        K::Literal(_) | K::Regex(_) | K::Tagged { .. } | K::Record(_) | K::Tuple(_) => None,
    }
}

fn node_json(source: &str, node: &Node) -> Result<Json, NativeError> {
    let value = match &node.value {
        Value::Null => Json::Null,
        Value::Bool(value) => Json::Bool(*value),
        Value::Number(span) => {
            let spelling = source.get(span.start..span.end).ok_or_else(|| {
                NativeError::new(
                    NativeErrorKind::InvalidDefinition,
                    "invalid number source span",
                )
                .at_span(*span)
            })?;
            serde_json::from_str(spelling).map_err(|error| {
                NativeError::new(
                    NativeErrorKind::InvalidDefinition,
                    format!("invalid number: {error}"),
                )
                .at_span(*span)
                .caused_by(error)
            })?
        }
        Value::String(StringValue::Decoded(text)) => Json::String(text.clone()),
        Value::String(StringValue::Source(span)) => {
            let text = source.get(span.start..span.end).ok_or_else(|| {
                NativeError::new(
                    NativeErrorKind::InvalidDefinition,
                    "invalid string source span",
                )
                .at_span(*span)
            })?;
            Json::String(text.to_owned())
        }
        Value::Temporal(value) => Json::String(
            value
                .value(source)
                .expect("admitted temporal source")
                .as_str()
                .to_owned(),
        ),
        Value::Sequence(items) => Json::Array(
            items
                .iter()
                .map(|item| node_json(source, item))
                .collect::<Result<_, _>>()?,
        ),
        Value::Mapping(entries) => {
            let mut object = Map::new();
            for entry in entries {
                let key = source.get(entry.key.start..entry.key.end).unwrap_or("");
                object.insert(key.into(), node_json(source, &entry.value)?);
            }
            Json::Object(object)
        }
    };
    Ok(value)
}

fn resolved_node<'a>(
    value: &Json,
    annotations: &[Json],
    location: &'a crate::SchemaLocation,
    path: &str,
    span: Span,
) -> Result<crate::ResolvedNode<'a>, NativeError> {
    use crate::resolved::ResolvedTag;
    use crate::{Origin, ResolvedNode, ResolvedValue};
    use std::{borrow::Cow, collections::BTreeMap};
    let origin = Origin::Default(location);
    let annotation = annotations.iter().find(|entry| {
        entry.get("tag").is_some() && entry.get("path").and_then(Json::as_str) == Some(path)
    });
    let tag = annotation.and_then(|entry| {
        Some(ResolvedTag {
            name: Cow::Owned(entry.get("tag")?.as_str()?.to_owned()),
            binding: entry
                .get("binding")
                .and_then(Json::as_str)
                .map(|binding| Cow::Owned(binding.to_owned())),
            origin,
            supplied: false,
        })
    });
    let resolved = match value {
        Json::Null => ResolvedValue::Null,
        Json::Bool(value) => ResolvedValue::Bool(*value),
        Json::Number(value) => ResolvedValue::Number(Cow::Owned(value.clone())),
        Json::String(value) => {
            if let Some(kind) = annotations
                .iter()
                .find(|entry| {
                    entry.get("kind").is_some()
                        && entry.get("path").and_then(Json::as_str) == Some(path)
                })
                .and_then(|entry| entry.get("kind"))
                .and_then(Json::as_str)
                .and_then(crate::TemporalKind::from_str)
            {
                ResolvedValue::Temporal(
                    crate::temporal::TemporalMetadata::canonical(kind, value)
                        .value(value)
                        .into_owned(),
                )
            } else {
                ResolvedValue::String(Cow::Owned(value.clone()))
            }
        }
        Json::Array(items) => ResolvedValue::Sequence(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    resolved_node(
                        item,
                        annotations,
                        location,
                        &format!("{path}/{index}"),
                        span,
                    )
                })
                .collect::<Result<_, _>>()?,
        ),
        Json::Object(fields) => {
            let mut resolved = BTreeMap::new();
            for (key, item) in fields {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                resolved.insert(
                    Cow::Owned(key.clone()),
                    resolved_node(
                        item,
                        annotations,
                        location,
                        &format!("{path}/{escaped}"),
                        span,
                    )?,
                );
            }
            ResolvedValue::Mapping(resolved)
        }
    };
    let _ = span;
    Ok(ResolvedNode {
        value: resolved,
        tag,
        origin,
        opaque: false,
        number_class: 0,
    })
}
fn definition_identity(root: &Path, relative: &Path) -> String {
    let root = root.as_os_str().as_encoded_bytes();
    let relative = relative.as_os_str().as_encoded_bytes();
    const H: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity((root.len() + relative.len() + 1) * 2);
    for byte in root
        .iter()
        .copied()
        .chain(std::iter::once(0))
        .chain(relative.iter().copied())
    {
        out.push(H[(byte >> 4) as usize] as char);
        out.push(H[(byte & 15) as usize] as char);
    }
    out
}
