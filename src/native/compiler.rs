use super::{Bundle, NativeError, NativeErrorKind};
use crate::{Document, Node, SchemaError, SchemaModule, Schemas, Span, StringValue, Value};
use serde_json::{Map, Number, Value as Json};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
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
    Normalization(usize),
    Validation(usize),
    Position(usize),
}

struct DefaultCheck {
    schema_type: String,
    path: PathBuf,
    span: Span,
    candidate_origin: String,
    fixed_choices: Vec<(String, usize)>,
}
#[derive(Clone)]
struct SchemaOrigin {
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
#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
enum DefaultPathStep {
    Property(String),
    Item(usize),
}
type DefaultPath = Vec<DefaultPathStep>;

// Formula atoms name original schema operations, not JSON-equivalent bodies.
// Only generated Boolean combinations distribute; source union choices keep
// their original registry identity and are selected by the existing resolver.
type DefaultMonomial = Vec<usize>;
type DefaultMemoState = Vec<DefaultMonomial>;

#[derive(Clone, Hash, PartialEq, Eq)]
struct DefaultScope {
    constraints: Rc<DefaultMemoState>,
    sources: Rc<DefaultMemoState>,
    fixed: Rc<Vec<(usize, usize)>>,
}

#[derive(Default)]
struct DefaultMemo {
    states: HashSet<DefaultScope>,
    empty_fixed: Rc<Vec<(usize, usize)>>,
}

struct DefaultPosition {
    root: usize,
    pointer: String,
    reference: String,
    children: Vec<DefaultChild>,
    referenced: Option<usize>,
    own: bool,
    reaches_default: bool,
}

struct DefaultChild {
    position: SchemaPosition,
    target: usize,
    applies: bool,
    unsupported_defaults: bool,
}

// These annotations exist only while lowering complete schema assertions. They
// are resolved and removed before either registry projection is admitted.
const PENDING_DEFAULTS: &str = "x-daml-native-pending-defaults";

struct PendingNormalization {
    schema: Json,
    bases: NormalizationBase,
    definition: Rc<super::Definition>,
    span: Span,
    file: PathBuf,
}

enum NormalizationBase {
    References(Vec<String>),
    Inherited {
        schemas: Vec<Json>,
        presets: Vec<PresetFragment>,
        root_replaced: bool,
    },
}

struct DefaultProjection {
    reference: String,
    paths: Vec<DefaultPath>,
}

struct PendingValidation {
    schema: Json,
    file: PathBuf,
    span: Span,
    container: bool,
}

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
    check_types: Map<String, Json>,
    validation_resources: Vec<Json>,
    default_roots: Vec<SchemaSource>,
    default_positions: Vec<DefaultPosition>,
    // Filled lazily only after source positions and their edges are frozen.
    // Exact ordered seeds retain original operation identity, including repeats.
    default_closures: HashMap<Vec<usize>, DefaultMonomial>,
    schema_origins: HashMap<String, SchemaOrigin>,
    next_regex_origin: u64,
    default_projections: HashMap<(String, Vec<DefaultPath>), String>,
    next_default_projection: usize,
    default_projection_sources: HashMap<String, Rc<DefaultProjection>>,
    pending_projections: Vec<String>,
    active_projections: HashSet<String>,
    pending_normalizations: Vec<Option<PendingNormalization>>,
    normalized_defaults: Vec<Option<Json>>,
    pending_validations: Vec<PendingValidation>,
    definition_stack: Vec<Rc<super::Definition>>,
    loading: bool,
    materialized_nodes: usize,
    materialized_bytes: usize,
    lowering_depth: usize,
    default_validation_steps: usize,
}

fn index_default_position(
    schema: &Json,
    root: (usize, &str),
    pointer: String,
    positions: &mut Vec<DefaultPosition>,
    sources: &mut HashMap<String, SchemaSource>,
    budget: &mut WalkBudget<'_>,
    depth: usize,
) -> Result<usize, NativeError> {
    budget.charge(depth)?;
    if positions.len() >= crate::schema::profile::MAX_SCHEMA_NODES {
        return Err(NativeError::new(
            NativeErrorKind::LimitExceeded,
            "native default source positions exceed schema-node bound",
        )
        .at_path(budget.file)
        .at_span(budget.span));
    }
    let reference_bytes = root.1.len().saturating_add(3 * pointer.len() + 1);
    admit_copy_cost(
        &mut budget.copied_nodes,
        &mut budget.copied_bytes,
        3,
        2 * reference_bytes + std::mem::size_of::<bool>(),
        budget.file,
        Some(budget.span),
    )?;
    let reference = if pointer.is_empty() {
        root.1.to_owned()
    } else {
        crate::schema::canonical_reference(root.1, &pointer)
    };
    let own = schema.as_object().is_none_or(|object| {
        object
            .keys()
            .any(|key| !matches!(key.as_str(), "$id" | "$ref" | "allOf"))
            || (object.get("$ref").is_none()
                && object
                    .get("allOf")
                    .and_then(Json::as_array)
                    .is_none_or(Vec::is_empty))
    });
    let id = positions.len();
    if let Some(source) = sources.get_mut(&reference) {
        *source = SchemaSource::Position(id);
    } else {
        sources.insert(reference.clone(), SchemaSource::Position(id));
    }
    positions.push(DefaultPosition {
        root: root.0,
        pointer,
        reference,
        children: Vec::new(),
        referenced: None,
        own,
        reaches_default: schema.get("default").is_some(),
    });
    for child in crate::schema::profile::children(schema) {
        let key_bytes = child.map_key().map_or(0, str::len);
        let pointer_bytes = positions[id]
            .pointer
            .len()
            .saturating_add(2 * (child.keyword().len() + key_bytes) + 32);
        admit_copy_cost(
            &mut budget.copied_nodes,
            &mut budget.copied_bytes,
            3,
            pointer_bytes + child.keyword().len() + key_bytes,
            budget.file,
            Some(budget.span),
        )?;
        let child_pointer = child.pointer(&positions[id].pointer);
        let position = SchemaPosition {
            keyword: child.keyword().to_owned(),
            key: child.map_key().map(str::to_owned),
            index: child.array_index(),
        };
        let target = index_default_position(
            child.value,
            root,
            child_pointer,
            positions,
            sources,
            budget,
            depth + 1,
        )?;
        positions[id].children.push(DefaultChild {
            position,
            target,
            applies: child.applies,
            unsupported_defaults: child.unsupported_defaults,
        });
    }
    Ok(id)
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
        check_types: Map::new(),
        validation_resources: Vec::new(),
        default_roots: Vec::new(),
        default_positions: Vec::new(),
        default_closures: HashMap::new(),
        schema_origins: HashMap::new(),
        next_regex_origin: 0,
        default_projections: HashMap::new(),
        next_default_projection: 0,
        default_projection_sources: HashMap::new(),
        pending_projections: Vec::new(),
        active_projections: HashSet::new(),
        pending_normalizations: Vec::new(),
        normalized_defaults: Vec::new(),
        pending_validations: Vec::new(),
        definition_stack: Vec::new(),
        loading: true,
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
    compiler.finish_defaults()?;
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
        let (mut validation_types, mut validation_resources) =
            relaxed_projection(&compiler.types, &compiler.resources);
        for (name, mut binding) in std::mem::take(&mut compiler.check_types) {
            relax_schema(binding.get_mut("schema").expect("default check binding"));
            validation_types.insert(name, binding);
        }
        for mut resource in std::mem::take(&mut compiler.validation_resources) {
            relax_schema(
                resource
                    .get_mut("schema")
                    .expect("default validation resource"),
            );
            validation_resources.push(resource);
        }
        let validation_value = Json::Object(Map::from_iter([
            ("types".into(), Json::Object(validation_types)),
            ("resources".into(), Json::Array(validation_resources)),
        ]));
        let validation_module = SchemaModule::from_value(validation_value)
            .map_err(|error| native_schema_error(error, &bundle.root, &compiler.schema_origins))?;
        Some(
            Schemas::compose([validation_module]).map_err(|error| {
                native_schema_error(error, &bundle.root, &compiler.schema_origins)
            })?,
        )
    };
    let module_value = Json::Object(Map::from_iter([
        ("types".into(), Json::Object(compiler.types)),
        ("resources".into(), Json::Array(compiler.resources)),
    ]));
    let module = SchemaModule::from_value(module_value)
        .map_err(|error| native_schema_error(error, &bundle.root, &compiler.schema_origins))?;
    let mut schemas = Schemas::compose([module])
        .map_err(|error| native_schema_error(error, &bundle.root, &compiler.schema_origins))?;
    schemas.restrict_source_tags(source_tags);
    let mut default_limits = crate::ResolutionLimits::default();
    for check in compiler.checks {
        let validation = validation_schemas
            .as_ref()
            .expect("default checks have a validation projection");
        let origin = validation
            .binding_schema(&check.candidate_origin)
            .ok_or_else(|| {
                NativeError::new(
                    NativeErrorKind::Schema,
                    "native default descriptor has no admitted source position",
                )
                .at_path(&check.path)
                .at_span(check.span)
            })?;
        let value = origin.value().get("default").ok_or_else(|| {
            NativeError::new(
                NativeErrorKind::Schema,
                "native default descriptor source has no own literal default",
            )
            .at_path(&check.path)
            .at_span(check.span)
        })?;
        let annotations = schema_default_annotations(
            origin
                .value()
                .as_object()
                .expect("literal default belongs to an object schema"),
        );
        admit_copy(
            &mut compiler.materialized_nodes,
            &mut compiler.materialized_bytes,
            value,
            &check.path,
            Some(check.span),
        )?;
        for annotation in annotations {
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
        let node = resolved_node(value, origin, &location);
        validation
            .complete_partial_default(
                &check.schema_type,
                node,
                &mut default_limits,
                &check.fixed_choices,
                &check.candidate_origin,
            )
            .map_err(|mut error| {
                if let Some(location) = error.schema.as_deref_mut() {
                    translate_validation_location(location, &compiler.schema_origins);
                }
                if let crate::resolved::ErrorOrigin::Default(location) = &mut error.origin {
                    translate_validation_location(location, &compiler.schema_origins);
                }
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
fn translate_validation_location(
    location: &mut crate::SchemaLocation,
    origins: &HashMap<String, SchemaOrigin>,
) {
    if location
        .resource
        .starts_with("urn:daml:native:default-validation:")
        && let Some(origin) = origins.get(&location.resource)
    {
        location.resource = format!("file://{}", origin.path.display());
    }
}

fn native_schema_error(
    mut error: SchemaError,
    fallback: &Path,
    origins: &HashMap<String, SchemaOrigin>,
) -> NativeError {
    let origin = error
        .location
        .as_ref()
        .and_then(|location| origins.get(&location.resource));
    if let Some(location) = &mut error.location {
        translate_validation_location(location, origins);
    }
    let kind = match error.kind {
        crate::SchemaErrorKind::ReferenceCycle => NativeErrorKind::ReferenceCycle,
        crate::SchemaErrorKind::LimitExceeded => NativeErrorKind::LimitExceeded,
        _ => NativeErrorKind::Schema,
    };
    let mut native = NativeError::new(kind, error.to_string()).caused_by(error);
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
    // Composite value literals remain atomic: their descendant presence
    // constraints are part of the literal, not an incomplete object preset.
    if object.get("x-daml-native-literal").and_then(Json::as_bool) == Some(true) {
        return;
    }
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
    fn finish_defaults(&mut self) -> Result<(), NativeError> {
        self.loading = false;
        let file = self.bundle.root.clone();
        let mut budget = WalkBudget {
            work: self.default_validation_steps,
            copied_nodes: self.materialized_nodes,
            copied_bytes: self.materialized_bytes,
            file: &file,
            span: Span { start: 0, end: 0 },
        };
        // Each operation has one stable identity and is resolved once. Schema
        // references stay symbolic across structural edges; no source stubs or
        // repeated whole-graph normalization passes are admitted.
        let mut ordinal = 0;
        while ordinal < self.pending_normalizations.len() || !self.pending_projections.is_empty() {
            if ordinal < self.pending_normalizations.len() {
                self.resolve_normalization(ordinal, &mut budget, 0)?;
                ordinal += 1;
            } else if let Some(reference) = self.pending_projections.pop() {
                self.resolve_projection(&reference, &mut budget, 0)?;
            }
        }
        for (ordinal, normalized) in std::mem::take(&mut self.normalized_defaults)
            .into_iter()
            .enumerate()
        {
            let mut schema = normalized.expect("all pending defaults resolved");
            let id = normalization_id(ordinal);
            schema
                .as_object_mut()
                .expect("normalized native schema is an object")
                .insert("$id".into(), Json::String(id.clone()));
            let index = self.resources.len();
            self.resources.push(Json::Object(Map::from_iter([
                ("uri".into(), Json::String(id.clone())),
                ("schema".into(), schema),
            ])));
            self.schema_sources
                .insert(id, SchemaSource::Resource(index));
        }
        // Materialize only schema positions, never literal/default data. A
        // normalized node becomes a reference to its complete private resource,
        // so recursive branch-local presets do not unroll schema structure.
        for binding in self.types.values_mut().chain(self.resources.iter_mut()) {
            materialize_normalizations(
                binding.get_mut("schema").expect("native schema binding"),
                &mut budget,
                0,
            )?;
        }
        let mut validations = std::mem::take(&mut self.pending_validations);
        for validation in &mut validations {
            let mut local_budget = WalkBudget {
                work: budget.work,
                copied_nodes: budget.copied_nodes,
                copied_bytes: budget.copied_bytes,
                file: &validation.file,
                span: validation.span,
            };
            materialize_normalizations(&mut validation.schema, &mut local_budget, 0)?;
            budget.work = local_budget.work;
            budget.copied_nodes = local_budget.copied_nodes;
            budget.copied_bytes = local_budget.copied_bytes;
        }
        let mut has_defaults = false;
        for schema in self
            .types
            .values()
            .chain(self.resources.iter())
            .map(|binding| binding.get("schema").expect("native schema binding"))
            .chain(validations.iter().map(|validation| &validation.schema))
        {
            if schema_has_default(schema, &mut budget, 0)? {
                has_defaults = true;
                break;
            }
        }
        self.default_validation_steps = budget.work;
        self.materialized_nodes = budget.copied_nodes;
        self.materialized_bytes = budget.copied_bytes;
        let mut validation_roots = Vec::new();
        for mut validation in validations {
            let mut budget = WalkBudget {
                work: self.default_validation_steps,
                copied_nodes: self.materialized_nodes,
                copied_bytes: self.materialized_bytes,
                file: &validation.file,
                span: validation.span,
            };
            if validation.container {
                if !self.schema_proves_container(
                    validation
                        .schema
                        .as_object()
                        .expect("native container schema"),
                    &validation.file,
                    validation.span,
                )? {
                    return Err(NativeError::new(
                        NativeErrorKind::InvalidDefault,
                        "record-field unset requires a container constraint",
                    )
                    .at_path(&validation.file)
                    .at_span(validation.span));
                }
            } else if has_defaults {
                let index = self.validation_resources.len();
                let reference = format!("urn:daml:native:default-validation:{index}");
                admit_copy_cost(
                    &mut budget.copied_nodes,
                    &mut budget.copied_bytes,
                    7,
                    4 * reference.len() + validation.file.as_os_str().len(),
                    &validation.file,
                    Some(validation.span),
                )?;
                validation
                    .schema
                    .as_object_mut()
                    .expect("native validation root is an object")
                    .insert("$id".into(), Json::String(reference.clone()));
                self.validation_resources.push(Json::Object(Map::from_iter([
                    ("uri".into(), Json::String(reference.clone())),
                    ("schema".into(), validation.schema),
                ])));
                self.schema_sources
                    .insert(reference.clone(), SchemaSource::Validation(index));
                self.schema_origins.insert(
                    reference.clone(),
                    SchemaOrigin {
                        path: validation.file.clone(),
                        span: validation.span,
                    },
                );
                self.default_validation_steps = budget.work;
                self.materialized_nodes = budget.copied_nodes;
                self.materialized_bytes = budget.copied_bytes;
                validation_roots.push((reference, validation.file, validation.span));
            }
        }
        if !has_defaults {
            return Ok(());
        }
        let mut budget = WalkBudget {
            work: self.default_validation_steps,
            copied_nodes: self.materialized_nodes,
            copied_bytes: self.materialized_bytes,
            file: &file,
            span: Span { start: 0, end: 0 },
        };
        self.prepare_default_positions(&mut budget)?;
        self.default_validation_steps = budget.work;
        self.materialized_nodes = budget.copied_nodes;
        self.materialized_bytes = budget.copied_bytes;
        for (reference, file, span) in validation_roots {
            let mut budget = WalkBudget {
                work: self.default_validation_steps,
                copied_nodes: self.materialized_nodes,
                copied_bytes: self.materialized_bytes,
                file: &file,
                span,
            };
            let position = self.default_position_id(&reference, &mut budget)?;
            let root = self.close_default_monomial(&[position], None, &mut budget, 0)?;
            admit_copy_cost(
                &mut budget.copied_nodes,
                &mut budget.copied_bytes,
                3,
                0,
                &file,
                Some(span),
            )?;
            let state = Rc::new(vec![root]);
            let mut seen = DefaultMemo::default();
            let scope = DefaultScope {
                constraints: Rc::clone(&state),
                sources: state,
                fixed: Rc::clone(&seen.empty_fixed),
            };
            self.visit_default_scope(scope, &mut budget, 0, &mut seen)?;
            self.default_validation_steps = budget.work;
            self.materialized_nodes = budget.copied_nodes;
            self.materialized_bytes = budget.copied_bytes;
        }
        Ok(())
    }

    fn prepare_default_positions(
        &mut self,
        budget: &mut WalkBudget<'_>,
    ) -> Result<(), NativeError> {
        let mut roots = Vec::new();
        for (reference, source) in &self.schema_sources {
            budget.charge(0)?;
            let source_bytes = match source {
                SchemaSource::Type(key) => key.len(),
                SchemaSource::Resource(_) | SchemaSource::Validation(_) => 0,
                SchemaSource::Normalization(_) | SchemaSource::Position(_) => {
                    unreachable!("default positions are prepared once after normalization")
                }
            };
            admit_copy_cost(
                &mut budget.copied_nodes,
                &mut budget.copied_bytes,
                2,
                reference.len() + source_bytes,
                budget.file,
                Some(budget.span),
            )?;
            roots.push((reference.clone(), source.clone()));
        }
        budget.work = budget.work.saturating_add(
            roots
                .len()
                .saturating_mul(roots.len().max(1).ilog2() as usize + 1),
        );
        budget.charge(0)?;
        roots.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        for (reference, source) in roots {
            let root = self.default_roots.len();
            let schema = match &source {
                SchemaSource::Type(key) => {
                    self.types.get(key).and_then(|entry| entry.get("schema"))
                }
                SchemaSource::Resource(index) => self.resources[*index].get("schema"),
                SchemaSource::Validation(index) => self.validation_resources[*index].get("schema"),
                SchemaSource::Normalization(_) | SchemaSource::Position(_) => unreachable!(),
            }
            .expect("admitted native root has a schema");
            index_default_position(
                schema,
                (root, &reference),
                String::new(),
                &mut self.default_positions,
                &mut self.schema_sources,
                budget,
                0,
            )?;
            self.default_roots.push(source);
        }
        for id in 0..self.default_positions.len() {
            budget.charge(0)?;
            let target = self.default_schema(id).get("$ref").and_then(Json::as_str);
            let referenced = target
                .map(|reference| self.default_position_id(reference, budget))
                .transpose()?;
            self.default_positions[id].referenced = referenced;
        }
        // Reachability is a least fixed point on the frozen source graph.
        // Include every schema-child edge conservatively; literal payloads are
        // not schema positions. Each position enters the worklist at most once.
        let count = self.default_positions.len();
        admit_copy_cost(
            &mut budget.copied_nodes,
            &mut budget.copied_bytes,
            count + 2,
            count * (std::mem::size_of::<Vec<usize>>() + std::mem::size_of::<usize>()),
            budget.file,
            Some(budget.span),
        )?;
        let mut reverse = (0..count).map(|_| Vec::new()).collect::<Vec<_>>();
        let mut pending = Vec::with_capacity(count);
        for (id, position) in self.default_positions.iter().enumerate() {
            budget.charge(0)?;
            if position.reaches_default {
                pending.push(id);
            }
            for target in position
                .referenced
                .into_iter()
                .chain(position.children.iter().map(|child| child.target))
            {
                budget.charge(0)?;
                admit_copy_cost(
                    &mut budget.copied_nodes,
                    &mut budget.copied_bytes,
                    1,
                    std::mem::size_of::<usize>(),
                    budget.file,
                    Some(budget.span),
                )?;
                reverse[target].push(id);
            }
        }
        while let Some(id) = pending.pop() {
            budget.charge(0)?;
            for &parent in &reverse[id] {
                budget.charge(0)?;
                if !self.default_positions[parent].reaches_default {
                    self.default_positions[parent].reaches_default = true;
                    pending.push(parent);
                }
            }
        }
        Ok(())
    }

    fn default_position_id(
        &self,
        reference: &str,
        budget: &mut WalkBudget<'_>,
    ) -> Result<usize, NativeError> {
        budget.charge(0)?;
        if let Some(SchemaSource::Position(id)) = self.schema_sources.get(reference) {
            Ok(*id)
        } else {
            Err(NativeError::new(
                NativeErrorKind::UnknownReference,
                "native default analysis cannot locate its source schema position",
            )
            .at_path(budget.file)
            .at_span(budget.span)
            .for_reference(reference))
        }
    }

    fn default_schema(&self, id: usize) -> &Json {
        let position = &self.default_positions[id];
        let schema = match &self.default_roots[position.root] {
            SchemaSource::Type(key) => self.types.get(key).and_then(|entry| entry.get("schema")),
            SchemaSource::Resource(index) => self.resources[*index].get("schema"),
            SchemaSource::Validation(index) => self.validation_resources[*index].get("schema"),
            SchemaSource::Normalization(_) | SchemaSource::Position(_) => unreachable!(),
        }
        .expect("prepared native root retains its schema");
        schema
            .pointer(&position.pointer)
            .expect("prepared native schema position exists")
    }

    fn close_default_monomial(
        &mut self,
        monomial: &[usize],
        arm: Option<usize>,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<DefaultMonomial, NativeError> {
        budget.charge(depth)?;
        if let Some(arm) = arm {
            // The sole arm caller supplies a ref/allOf-closed parent state.
            // cl(C ∪ {arm}) = C ∪ cl({arm}); reuse the same frozen owner.
            let closed = self.fresh_default_closure(&[arm], budget, depth)?;
            merge_default_monomials(monomial, closed, budget, depth)
        } else {
            let closed = self.fresh_default_closure(monomial, budget, depth)?;
            copy_default_monomial(closed, budget, depth)
        }
    }

    fn fresh_default_closure(
        &mut self,
        monomial: &[usize],
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<&DefaultMonomial, NativeError> {
        budget.work = budget.work.saturating_add(monomial.len());
        budget.charge(depth)?;
        if !self.default_closures.contains_key(monomial) {
            charge_default_monomial(monomial.len(), budget, depth)?;
            let mut pending = Vec::with_capacity(monomial.len());
            pending.extend_from_slice(monomial);
            let mut visited = HashSet::new();
            let mut closed = Vec::new();
            while let Some(id) = pending.pop() {
                budget.charge(depth)?;
                if visited.contains(&id) {
                    continue;
                }
                charge_default_monomial(1, budget, depth)?;
                visited.insert(id);
                let position = &self.default_positions[id];
                if position.own {
                    closed.push(id);
                }
                if let Some(target) = position.referenced {
                    charge_default_monomial(1, budget, depth)?;
                    pending.push(target);
                }
                for child in &position.children {
                    if child.position.keyword == "allOf" {
                        charge_default_monomial(1, budget, depth)?;
                        pending.push(child.target);
                    }
                }
            }
            charge_default_monomial(closed.len(), budget, depth)?;
            closed.sort_unstable();
            // Store only complete successful closures. Neither errors nor their
            // budget/origin context are reusable; hits pay their current context.
            budget.work = budget.work.saturating_add(2 * monomial.len());
            budget.charge(depth)?;
            admit_copy_cost(
                &mut budget.copied_nodes,
                &mut budget.copied_bytes,
                monomial.len() + 2,
                std::mem::size_of::<(Vec<usize>, DefaultMonomial)>()
                    + std::mem::size_of_val(monomial),
                budget.file,
                Some(budget.span),
            )?;
            self.default_closures.insert(monomial.to_vec(), closed);
        }
        budget.work = budget.work.saturating_add(monomial.len());
        budget.charge(depth)?;
        Ok(self
            .default_closures
            .get(monomial)
            .expect("successful fresh closure is retained"))
    }

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
        if self.types.contains_key(&key) || self.active.contains(&key) {
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
        self.active.insert(key.clone());
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
        let definition = Rc::new(definition);
        self.definition_stack.push(Rc::clone(&definition));
        let (mut contract, mut full) = self.lower_definition(
            &definition.root,
            definition.source(),
            &canonical_root,
            &canonical_file,
        )?;
        self.definition_stack.pop();
        let contract_id = format!("{id}:contract");
        contract.insert("$id".into(), Json::String(contract_id.clone()));
        let origin = SchemaOrigin {
            path: canonical_file.clone(),
            span: definition.root.span,
        };
        self.schema_origins
            .insert(contract_id.clone(), origin.clone());
        self.schema_origins.insert(id.clone(), origin);
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
        budget: Option<&mut WalkBudget<'_>>,
    ) -> Result<String, NativeError> {
        // Removing defaults commutes and is idempotent, but ancestor paths do
        // not subsume descendants: child-schema defaults still need projecting.
        let prior = self.default_projection_sources.get(reference);
        let insertion = match prior.map(|projection| {
            projection
                .paths
                .binary_search_by(|candidate| candidate.as_slice().cmp(path))
        }) {
            Some(Ok(_)) => return Ok(reference.to_owned()),
            Some(Err(index)) => index,
            None => 0,
        };
        let source = prior.map_or(reference, |projection| projection.reference.as_str());
        let paths = prior.map_or(&[][..], |projection| projection.paths.as_slice());
        let path_nodes = paths.iter().map(Vec::len).sum::<usize>() + path.len();
        let path_bytes = paths
            .iter()
            .flatten()
            .chain(path)
            .fold(0usize, |bytes, step| {
                bytes.saturating_add(match step {
                    DefaultPathStep::Property(key) => key.len(),
                    DefaultPathStep::Item(_) => std::mem::size_of::<usize>(),
                })
            });
        let nodes = 2 + paths.len() + path_nodes;
        let bytes = source.len() + path_bytes;
        let origin_span = budget.as_ref().map(|budget| budget.span).or_else(|| {
            self.definition_stack
                .last()
                .map(|definition| definition.root.span)
        });
        let (copied_nodes, copied_bytes) = match budget {
            Some(budget) => (&mut budget.copied_nodes, &mut budget.copied_bytes),
            None => (&mut self.materialized_nodes, &mut self.materialized_bytes),
        };
        admit_copy_cost(copied_nodes, copied_bytes, nodes, bytes, file, None)?;
        let mut paths = paths.to_vec();
        paths.insert(insertion, path.to_vec());
        let key = (source.to_owned(), paths);
        if let Some(projected) = self.default_projections.get(&key) {
            return Ok(projected.clone());
        }
        if self.next_default_projection >= crate::schema::profile::MAX_SCHEMA_NODES {
            return Err(NativeError::new(
                NativeErrorKind::LimitExceeded,
                "native default projection exceeds schema-node bound",
            )
            .at_path(file));
        }
        let variant = self.next_default_projection;
        self.next_default_projection += 1;
        let id = format!("urn:daml:native:default-projection:{variant}");
        // Retain the recipe for canonical composition after resolution. The
        // queue owns only an ID; schema_sources remains the completion owner.
        admit_copy_cost(
            copied_nodes,
            copied_bytes,
            nodes + 5,
            bytes + 4 * id.len(),
            file,
            None,
        )?;
        let projection = Rc::new(DefaultProjection {
            reference: key.0.clone(),
            paths: key.1.clone(),
        });
        self.default_projections.insert(key, id.clone());
        self.default_projection_sources
            .insert(id.clone(), projection);
        self.pending_projections.push(id.clone());
        if let Some(span) = origin_span {
            self.schema_origins.insert(
                id.clone(),
                SchemaOrigin {
                    path: file.to_owned(),
                    span,
                },
            );
        }
        Ok(id)
    }

    fn resolve_projection(
        &mut self,
        reference: &str,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        budget.charge(depth)?;
        if self.schema_sources.contains_key(reference) {
            return Ok(());
        }
        if self.active_projections.contains(reference) {
            return Err(NativeError::new(
                NativeErrorKind::ReferenceCycle,
                "default projection follows a non-progressing reference cycle",
            )
            .at_path(budget.file)
            .at_span(budget.span));
        }
        let Some(pending) = self
            .default_projection_sources
            .get(reference)
            .map(Rc::clone)
        else {
            return Ok(());
        };
        self.active_projections.insert(reference.to_owned());
        let mut schema = self.schema_for_reference(&pending.reference, budget, depth + 1)?;
        let file = budget.file;
        for path in &pending.paths {
            self.project_schema_defaults(&mut schema, path, file, budget, depth + 1)?;
        }
        schema
            .as_object_mut()
            .expect("native schemas are objects")
            .insert("$id".into(), Json::String(reference.to_owned()));
        let index = self.resources.len();
        self.resources.push(Json::Object(Map::from_iter([
            ("uri".into(), Json::String(reference.to_owned())),
            ("schema".into(), schema),
        ])));
        self.schema_sources
            .insert(reference.to_owned(), SchemaSource::Resource(index));
        self.active_projections.remove(reference);
        Ok(())
    }

    fn project_schema_defaults(
        &mut self,
        schema: &mut Json,
        path: &[DefaultPathStep],
        file: &Path,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<bool, NativeError> {
        budget.charge(depth)?;
        if schema.get(PENDING_DEFAULTS).is_some() {
            // Projection consumes an instance path, not a normalization
            // dependency. Keep that dependency symbolic until its owner runs.
            materialize_normalizations(schema, budget, depth)?;
        }
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
            let projected = self.project_default_reference(&reference, path, file, Some(budget))?;
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
            budget.charge_copy(items)?;
            let items = items.clone();
            let prefix = schema
                .as_object_mut()
                .expect("checked object")
                .entry("prefixItems")
                .or_insert_with(|| Json::Array(Vec::new()))
                .as_array_mut()
                .expect("native prefixItems is an array");
            while prefix.len() <= index {
                budget.charge_copy(&items)?;
                let child = items.clone();
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
            budget.charge_copy(child)?;
            let mut projected = child.clone();
            if self.project_schema_defaults(&mut projected, &path[1..], file, budget, depth + 1)? {
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
                    match self.project_schema_defaults(child, child_path, file, budget, depth + 1) {
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
                    let K::Regex { pattern, payload } = &member.constraint.kind else {
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
                    // The admitted token retains its original delimiters; diagnostics
                    // refer to the original interior, never offsets into laid-out text.
                    let delimiter_bytes = if source[payload.start..].starts_with("\"\"\"")
                        || source[payload.start..].starts_with("'''")
                    {
                        3
                    } else {
                        1
                    };
                    self.schema_origins.insert(
                        crate::schema::native_regex_origin(origin),
                        SchemaOrigin {
                            path: file.to_path_buf(),
                            span: Span {
                                start: payload.start + delimiter_bytes,
                                end: payload.end - delimiter_bytes,
                            },
                        },
                    );
                    schema.insert(
                        "x-daml-regex".into(),
                        Json::Object(Map::from_iter([
                            (
                                "pattern".into(),
                                Json::String(match pattern {
                                    StringValue::Source(span) => {
                                        source[span.start..span.end].to_owned()
                                    }
                                    StringValue::Decoded(text) => text.clone(),
                                }),
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
                        base_schemas.push(reference.to_owned());
                        let mut projected = reference.to_owned();
                        for path in &tuple_overrides {
                            projected = self.project_default_reference(
                                &projected,
                                path,
                                context.file,
                                None,
                            )?;
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
                        self.defer_normalization(
                            base_schemas.clone(),
                            schema,
                            operand,
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
                "String"
                    | "Int"
                    | "Number"
                    | "List"
                    | "Map"
                    | "Date"
                    | "Time"
                    | "DateTime"
                    | "Cron"
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
        let base = body
            .is_some_and(supports_default_override_projection)
            .then(|| id.clone());
        let mut target = if suppress_children {
            format!("{id}:contract")
        } else {
            id
        };
        if let Some(body) = body {
            let mut paths = Vec::new();
            collect_default_override_paths(body, source, &mut Vec::new(), &mut paths);
            for path in paths {
                target = self.project_default_reference(&target, &path, file, None)?;
            }
        }
        let mut schema = Map::from_iter([("$ref".into(), Json::String(target))]);
        if let Some(body) = body {
            let narrowed =
                self.lower_expression(body, context, suppress_children, validate_children)?;
            let mut narrowed = Json::Object(narrowed);
            if let Some(base) = &base {
                self.defer_normalization(vec![base.clone()], &mut narrowed, body, file)?;
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
        if !items.is_empty() {
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
        }
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
        let mut schema = self.lower_literal_shape(node, source, file)?;
        if matches!(&node.value, Value::Sequence(_) | Value::Mapping(_)) {
            // Keep exact presence throughout the literal's typed shape without
            // imposing JSON string equality on intrinsic temporal descendants.
            admit_copy_cost(
                &mut self.materialized_nodes,
                &mut self.materialized_bytes,
                1,
                "x-daml-native-literal".len() + std::mem::size_of::<bool>(),
                file,
                Some(node.span),
            )?;
            schema.insert("x-daml-native-literal".into(), Json::Bool(true));
        }
        Ok(schema)
    }

    fn lower_literal_shape(
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
                schema.insert("type".into(), Json::String("array".into()));
                if !items.is_empty() {
                    let children = items
                        .iter()
                        .map(|item| {
                            self.lower_literal_shape(item, source, file)
                                .map(Json::Object)
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    schema.insert("prefixItems".into(), Json::Array(children));
                }
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
                        Json::Object(self.lower_literal_shape(&entry.value, source, file)?),
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
            K::Regex { .. } => {
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
        // Lowering snapshots are admitted only after every symbolic operation
        // has finished and their original schema positions can be referenced.
        admit_copy(
            &mut self.materialized_nodes,
            &mut self.materialized_bytes,
            schema,
            file,
            Some(span),
        )?;
        self.pending_validations.push(PendingValidation {
            schema: schema.clone(),
            file: file.to_owned(),
            span,
            container: false,
        });
        Ok(())
    }
    fn schema_proves_container(
        &mut self,
        schema: &Map<String, Json>,
        file: &Path,
        span: Span,
    ) -> Result<bool, NativeError> {
        if self.loading {
            let (nodes, bytes) = json_map_cost(schema);
            admit_copy_cost(
                &mut self.materialized_nodes,
                &mut self.materialized_bytes,
                nodes,
                bytes,
                file,
                Some(span),
            )?;
            self.pending_validations.push(PendingValidation {
                schema: Json::Object(schema.clone()),
                file: file.to_owned(),
                span,
                container: true,
            });
            return Ok(true);
        }
        let mut budget = WalkBudget {
            work: self.default_validation_steps,
            copied_nodes: self.materialized_nodes,
            copied_bytes: self.materialized_bytes,
            file,
            span,
        };
        let result =
            self.schema_proves_container_inner(schema, &mut budget, 0, &mut HashSet::new());
        self.default_validation_steps = budget.work;
        self.materialized_nodes = budget.copied_nodes;
        self.materialized_bytes = budget.copied_bytes;
        result
    }

    fn schema_for_reference(
        &mut self,
        reference: &str,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<Json, NativeError> {
        self.resolve_projection(reference, budget, depth)?;
        if let Some(SchemaSource::Normalization(ordinal)) = self.schema_sources.get(reference) {
            self.resolve_normalization(*ordinal, budget, depth)?;
        }
        let schema = match self.schema_sources.get(reference) {
            Some(SchemaSource::Type(key)) => {
                self.types.get(key).and_then(|entry| entry.get("schema"))
            }
            Some(SchemaSource::Resource(index)) => self
                .resources
                .get(*index)
                .and_then(|entry| entry.get("schema")),
            Some(SchemaSource::Normalization(ordinal)) => {
                self.normalized_defaults[*ordinal].as_ref()
            }
            Some(SchemaSource::Validation(index)) => self
                .validation_resources
                .get(*index)
                .and_then(|entry| entry.get("schema")),
            Some(SchemaSource::Position(id)) => Some(self.default_schema(*id)),
            None => None,
        }
        .ok_or_else(|| {
            NativeError::new(
                NativeErrorKind::UnknownReference,
                "native default traversal cannot resolve its schema reference",
            )
            .at_path(budget.file)
            .at_span(budget.span)
            .for_reference(reference)
        })?;
        budget.charge_copy(schema)?;
        Ok(schema.clone())
    }

    fn schema_proves_container_inner(
        &mut self,
        schema: &Map<String, Json>,
        budget: &mut WalkBudget<'_>,
        depth: usize,
        active: &mut HashSet<String>,
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
            if !active.insert(reference.to_owned()) {
                return Err(NativeError::new(
                    NativeErrorKind::ReferenceCycle,
                    "container proof follows a non-progressing reference cycle",
                )
                .at_path(budget.file)
                .at_span(budget.span)
                .for_reference(reference));
            }
            let target = self.schema_for_reference(reference, budget, depth + 1)?;
            let proof = if let Some(target) = target.as_object() {
                self.schema_proves_container_inner(target, budget, depth + 1, active)
            } else {
                Ok(false)
            };
            active.remove(reference);
            if proof? {
                return Ok(true);
            }
        }
        if let Some(branches) = schema.get("allOf").and_then(Json::as_array) {
            for branch in branches {
                if let Some(branch) = branch.as_object()
                    && self.schema_proves_container_inner(branch, budget, depth + 1, active)?
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
                    if !self.schema_proves_container_inner(branch, budget, depth + 1, active)? {
                        return Ok(false);
                    }
                }
                has_alternatives = true;
            }
        }
        Ok(has_alternatives)
    }

    fn visit_default_scope(
        &mut self,
        scope: DefaultScope,
        budget: &mut WalkBudget<'_>,
        depth: usize,
        seen: &mut DefaultMemo,
    ) -> Result<(), NativeError> {
        if !self.default_sources_reach_default(&scope.sources, budget, depth)? {
            return Ok(());
        }
        let identity_work = scope
            .constraints
            .iter()
            .chain(scope.sources.iter())
            .map(|term| term.len().saturating_add(1))
            .sum::<usize>()
            .saturating_add(2 * scope.fixed.len());
        budget.work = budget.work.saturating_add(identity_work);
        budget.charge(depth)?;
        if seen.states.contains(&scope) {
            return Ok(());
        }
        admit_copy_cost(
            &mut budget.copied_nodes,
            &mut budget.copied_bytes,
            1,
            std::mem::size_of::<DefaultScope>(),
            budget.file,
            Some(budget.span),
        )?;
        budget.work = budget.work.saturating_add(identity_work);
        budget.charge(depth)?;
        seen.states.insert(scope.clone());
        for source in scope.sources.iter() {
            budget.work = budget.work.saturating_add(source.len());
            budget.charge(depth)?;
            let has_work = source.iter().any(|id| {
                self.default_schema(*id).get("default").is_some()
                    || (scope
                        .fixed
                        .binary_search_by_key(id, |choice| choice.0)
                        .is_err()
                        && self.default_positions[*id]
                            .children
                            .iter()
                            .any(|child| child.position.keyword == "anyOf"))
            });
            if !has_work {
                continue;
            }
            let supported =
                supporting_default_constraints(&scope.constraints, source, budget, depth)?;
            if supported.is_empty() {
                return Err(default_invariant(
                    budget,
                    "a candidate source has no supporting constraint term",
                ));
            }
            for id in source {
                budget.charge(depth)?;
                if self.default_schema(*id).get("default").is_some() {
                    self.record_default_check(&supported, *id, &scope.fixed, budget, depth)?;
                }
            }
            for id in source {
                let insertion = match scope.fixed.binary_search_by_key(id, |choice| choice.0) {
                    Ok(_) => continue,
                    Err(insertion) => insertion,
                };
                for child_index in 0..self.default_positions[*id].children.len() {
                    budget.charge(depth)?;
                    let child = &self.default_positions[*id].children[child_index];
                    if child.position.keyword != "anyOf" {
                        continue;
                    }
                    let target = child.target;
                    let arm = child.position.index.expect("anyOf children have indices");
                    let branch = self.close_default_monomial(&[target], None, budget, depth)?;
                    if !self.default_sources_reach_default(
                        std::slice::from_ref(&branch),
                        budget,
                        depth,
                    )? {
                        continue;
                    }
                    admit_copy_cost(
                        &mut budget.copied_nodes,
                        &mut budget.copied_bytes,
                        3 + scope.fixed.len(),
                        (scope.fixed.len() + 1) * 2 * std::mem::size_of::<usize>(),
                        budget.file,
                        Some(budget.span),
                    )?;
                    let sources = Rc::new(vec![branch]);
                    let constraints =
                        Rc::new(default_memo_product(&supported, &sources, budget, depth)?);
                    let mut fixed = scope.fixed.as_ref().clone();
                    fixed.insert(insertion, (*id, arm));
                    self.visit_default_scope(
                        DefaultScope {
                            constraints,
                            sources,
                            fixed: Rc::new(fixed),
                        },
                        budget,
                        depth + 1,
                        seen,
                    )?;
                }
            }
        }
        // Branch recursion owns the structural walk of its selected source
        // arm. Outside/common sources derive only their own direct children;
        // unrelated union alternatives remain existential constraints.
        for position in self.default_child_queries(&scope.sources, budget, depth)? {
            let Some(sources) =
                self.derive_own_default_state(&scope.sources, &position, budget, depth)?
            else {
                continue;
            };
            if !self.default_sources_reach_default(&sources, budget, depth)? {
                continue;
            }
            let constraints = self
                .derive_default_state(&scope.constraints, &position, &scope.fixed, budget, depth)?
                .ok_or_else(|| {
                    default_invariant(budget, "a source child has no effective constraint child")
                })?;
            admit_copy_cost(
                &mut budget.copied_nodes,
                &mut budget.copied_bytes,
                2,
                0,
                budget.file,
                Some(budget.span),
            )?;
            let constraints = Rc::new(constraints);
            let sources = Rc::new(sources);
            self.visit_default_scope(
                DefaultScope {
                    constraints,
                    sources,
                    fixed: Rc::clone(&seen.empty_fixed),
                },
                budget,
                depth + 1,
                seen,
            )?;
        }
        Ok(())
    }

    fn default_child_queries(
        &self,
        state: &DefaultMemoState,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<Vec<SchemaPosition>, NativeError> {
        // Inputs are already globally ref/allOf-closed. AnyOf source arms are
        // visited separately with their own root assignments.
        let mut visited = HashSet::new();
        let mut queries = HashSet::new();
        for id in state.iter().flatten().copied() {
            budget.charge(depth)?;
            if visited.contains(&id) {
                continue;
            }
            charge_default_monomial(1, budget, depth)?;
            visited.insert(id);
            let position = &self.default_positions[id];
            for child in &position.children {
                budget.charge(depth)?;
                if !child.applies || child.unsupported_defaults {
                    continue;
                }
                let query = match child.position.keyword.as_str() {
                    "allOf" | "anyOf" | "oneOf" => continue,
                    "properties" => ("property", child.position.key.as_deref(), None),
                    "additionalProperties" => ("property", None, None),
                    "prefixItems" => ("item", None, child.position.index),
                    "items" => ("item", None, None),
                    keyword if child.position.key.is_some() || child.position.index.is_some() => {
                        (keyword, child.position.key.as_deref(), child.position.index)
                    }
                    _ => continue,
                };
                if !queries.contains(&query) {
                    admit_copy_cost(
                        &mut budget.copied_nodes,
                        &mut budget.copied_bytes,
                        1,
                        std::mem::size_of::<SchemaPosition>(),
                        budget.file,
                        Some(budget.span),
                    )?;
                    queries.insert(query);
                }
            }
        }
        let mut result = Vec::with_capacity(queries.len());
        for (keyword, key, index) in queries {
            admit_copy_cost(
                &mut budget.copied_nodes,
                &mut budget.copied_bytes,
                1,
                keyword.len() + key.map_or(0, str::len),
                budget.file,
                Some(budget.span),
            )?;
            result.push(SchemaPosition {
                keyword: keyword.to_owned(),
                key: key.map(str::to_owned),
                index,
            });
        }
        budget.work = budget.work.saturating_add(
            result
                .len()
                .saturating_mul(result.len().max(1).ilog2() as usize + 1),
        );
        budget.charge(depth)?;
        result.sort_unstable_by(|left, right| {
            (&left.keyword, &left.key, left.index).cmp(&(&right.keyword, &right.key, right.index))
        });
        Ok(result)
    }

    fn selected_default_child(&self, id: usize, query: &SchemaPosition) -> Option<usize> {
        let children = &self.default_positions[id].children;
        let find = |keyword: &str, key: Option<&str>, index: Option<usize>| {
            children
                .iter()
                .find(|child| {
                    child.position.keyword == keyword
                        && child.position.key.as_deref() == key
                        && child.position.index == index
                })
                .map(|child| child.target)
        };
        match query.keyword.as_str() {
            "property" => query
                .key
                .as_deref()
                .and_then(|key| find("properties", Some(key), None))
                .or_else(|| {
                    self.default_schema(id)
                        .get("patternProperties")
                        .is_none()
                        .then(|| find("additionalProperties", None, None))
                        .flatten()
                }),
            "item" => query
                .index
                .and_then(|index| find("prefixItems", None, Some(index)))
                .or_else(|| {
                    let prefix = self
                        .default_schema(id)
                        .get("prefixItems")
                        .and_then(Json::as_array)
                        .map_or(0, Vec::len);
                    query
                        .index
                        .is_none_or(|index| index >= prefix)
                        .then(|| find("items", None, None))
                        .flatten()
                }),
            keyword => find(keyword, query.key.as_deref(), query.index),
        }
    }

    fn default_sources_reach_default(
        &self,
        sources: &[DefaultMonomial],
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<bool, NativeError> {
        for id in sources.iter().flatten() {
            budget.charge(depth)?;
            if self.default_positions[*id].reaches_default {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn derive_own_default_state(
        &mut self,
        state: &DefaultMemoState,
        query: &SchemaPosition,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<Option<DefaultMemoState>, NativeError> {
        let mut result = Vec::new();
        let mut any_child = false;
        for monomial in state {
            let mut children = Vec::new();
            for id in monomial {
                budget.charge(depth)?;
                if let Some(child) = self.selected_default_child(*id, query) {
                    charge_default_monomial(1, budget, depth)?;
                    children.push(child);
                }
            }
            if children.is_empty() {
                charge_default_monomial(0, budget, depth)?;
                result.push(Vec::new());
            } else {
                any_child = true;
                result.push(self.close_default_monomial(&children, None, budget, depth + 1)?);
            }
        }
        if !any_child {
            return Ok(None);
        }
        canonical_default_monomials(&mut result, budget, depth)?;
        Ok(Some(result))
    }

    fn derive_default_state(
        &mut self,
        state: &DefaultMemoState,
        query: &SchemaPosition,
        root_fixed: &[(usize, usize)],
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<Option<DefaultMemoState>, NativeError> {
        let mut result = Vec::new();
        let mut any_child = false;
        for monomial in state {
            // Every producer stores exact sorted ref/allOf-closed monomials.
            let closed = copy_default_monomial(monomial, budget, depth)?;
            admit_copy_cost(
                &mut budget.copied_nodes,
                &mut budget.copied_bytes,
                1 + root_fixed.len(),
                2 * root_fixed.len() * std::mem::size_of::<usize>(),
                budget.file,
                Some(budget.span),
            )?;
            for (id, _) in root_fixed {
                budget.work = budget
                    .work
                    .saturating_add(closed.len().max(1).ilog2() as usize + 1);
                budget.charge(depth)?;
                if closed.binary_search(id).is_err() {
                    return Err(default_invariant(
                        budget,
                        "a parent assignment is absent from its constraint term",
                    ));
                }
            }
            let mut pending = vec![(closed, root_fixed.to_vec())];
            while let Some((current, fixed)) = pending.pop() {
                budget.charge(depth)?;
                let union = current
                    .iter()
                    .find(|id| {
                        fixed.binary_search_by_key(*id, |choice| choice.0).is_err()
                            && self.default_positions[**id]
                                .children
                                .iter()
                                .any(|child| child.position.keyword == "anyOf")
                    })
                    .copied();
                if let Some(union) = union {
                    let insertion = fixed
                        .binary_search_by_key(&union, |choice| choice.0)
                        .expect_err("an unselected union has no parent assignment");
                    for child_index in 0..self.default_positions[union].children.len() {
                        budget.charge(depth)?;
                        let child = &self.default_positions[union].children[child_index];
                        if child.position.keyword != "anyOf" {
                            continue;
                        }
                        let target = child.target;
                        let arm = child.position.index.expect("anyOf arm index");
                        let branch =
                            self.close_default_monomial(&current, Some(target), budget, depth)?;
                        admit_copy_cost(
                            &mut budget.copied_nodes,
                            &mut budget.copied_bytes,
                            1 + fixed.len(),
                            2 * (fixed.len() + 1) * std::mem::size_of::<usize>(),
                            budget.file,
                            Some(budget.span),
                        )?;
                        let mut choices = fixed.clone();
                        choices.insert(insertion, (union, arm));
                        pending.push((branch, choices));
                    }
                    continue;
                }
                let mut children = Vec::new();
                for id in current {
                    budget.charge(depth)?;
                    if let Some(child) = self.selected_default_child(id, query) {
                        charge_default_monomial(1, budget, depth)?;
                        children.push(child);
                    }
                }
                if children.is_empty() {
                    // Keep mixed absent alternatives, but never manufacture a
                    // child when every reachable parent assignment lacks one.
                    charge_default_monomial(0, budget, depth)?;
                    result.push(Vec::new());
                } else {
                    any_child = true;
                    // Parent choices stop here. The same original operation
                    // below a data edge has independent choices at that child.
                    result.push(self.close_default_monomial(&children, None, budget, depth + 1)?);
                }
            }
        }
        if !any_child {
            return Ok(None);
        }
        canonical_default_monomials(&mut result, budget, depth)?;
        Ok(Some(result))
    }

    fn effective_schema_child(
        &mut self,
        schema: &Json,
        position: &SchemaPosition,
        visited: &mut HashSet<String>,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<Option<Json>, NativeError> {
        budget.charge(depth)?;
        let normalized = self.normalized_schema(schema, budget, depth)?;
        let schema = normalized.as_ref().unwrap_or(schema);
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
            let target = self.schema_for_reference(reference, budget, depth + 1)?;
            if let Some(child) =
                self.effective_schema_child(&target, position, visited, budget, depth + 1)?
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
        let mut any_union_child = false;
        if let Some(branches) = schema.get("anyOf").and_then(Json::as_array) {
            for branch in branches {
                let child = self.effective_schema_child(
                    branch,
                    position,
                    &mut visited.clone(),
                    budget,
                    depth + 1,
                )?;
                any_union_child |= child.is_some();
                union.push(child.unwrap_or(Json::Bool(true)));
            }
        }
        // Preserve absence when no arm declares a child at this position.
        // Mixed arms still need their true placeholders, and an explicitly
        // declared true/false child remains a real projection.
        if any_union_child {
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
        &mut self,
        schema: &Json,
        visited: &mut HashSet<String>,
        found: &mut Vec<(Json, Vec<Json>)>,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        budget.charge(depth)?;
        let normalized = self.normalized_schema(schema, budget, depth)?;
        let schema = normalized.as_ref().unwrap_or(schema);
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
                let target = self.schema_for_reference(reference, budget, depth + 1)?;
                self.collect_unconditional_defaults(&target, visited, found, budget, depth + 1)?;
            }
            if let Some(all_of) = object.get("allOf").and_then(Json::as_array) {
                for branch in all_of {
                    self.collect_unconditional_defaults(branch, visited, found, budget, depth + 1)?;
                }
            }
        }
        Ok(())
    }
    fn defer_normalization(
        &mut self,
        bases: Vec<String>,
        narrowed: &mut Json,
        expression: &crate::native::Expression,
        file: &Path,
    ) -> Result<(), NativeError> {
        if !contains_union(expression) {
            return Ok(());
        }
        admit_copy(
            &mut self.materialized_nodes,
            &mut self.materialized_bytes,
            narrowed,
            file,
            Some(expression.span),
        )?;
        let ordinal = self.pending_normalizations.len();
        if ordinal >= crate::schema::profile::MAX_SCHEMA_NODES {
            return Err(NativeError::new(
                NativeErrorKind::LimitExceeded,
                "native deferred defaults exceed schema-node bound",
            )
            .at_path(file)
            .at_span(expression.span));
        }
        let id = normalization_id(ordinal);
        admit_copy_cost(
            &mut self.materialized_nodes,
            &mut self.materialized_bytes,
            bases.len() + 3,
            bases.iter().map(String::len).sum::<usize>() + PENDING_DEFAULTS.len() + id.len(),
            file,
            Some(expression.span),
        )?;
        self.pending_normalizations.push(Some(PendingNormalization {
            schema: narrowed.clone(),
            bases: NormalizationBase::References(bases),
            definition: Rc::clone(
                self.definition_stack
                    .last()
                    .expect("lowering owns its definition"),
            ),
            span: expression.span,
            file: file.to_owned(),
        }));
        self.normalized_defaults.push(None);
        self.schema_sources
            .insert(id, SchemaSource::Normalization(ordinal));
        narrowed
            .as_object_mut()
            .expect("native narrowing is an object schema")
            .insert(PENDING_DEFAULTS.into(), Json::from(ordinal));
        Ok(())
    }

    fn resolve_normalization(
        &mut self,
        ordinal: usize,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        budget.charge(depth)?;
        if self.normalized_defaults[ordinal].is_some() {
            return Ok(());
        }
        let pending = self.pending_normalizations[ordinal].take().ok_or_else(|| {
            NativeError::new(
                NativeErrorKind::ReferenceCycle,
                "native default normalization follows a non-progressing reference cycle",
            )
            .at_path(budget.file)
            .at_span(budget.span)
        })?;
        self.schema_origins.insert(
            normalization_id(ordinal),
            SchemaOrigin {
                path: pending.file.clone(),
                span: pending.span,
            },
        );
        let expression = expression_at(&pending.definition.root, pending.span)
            .expect("deferred normalization retains its original expression");
        let mut local_budget = WalkBudget {
            work: budget.work,
            copied_nodes: budget.copied_nodes,
            copied_bytes: budget.copied_bytes,
            file: &pending.file,
            span: pending.span,
        };
        let (bases, presets, root_replaced) = match pending.bases {
            NormalizationBase::References(references) => {
                let mut bases = Vec::with_capacity(references.len());
                for reference in references {
                    bases.push(self.schema_for_reference(
                        &reference,
                        &mut local_budget,
                        depth + 1,
                    )?);
                }
                (bases, Vec::new(), false)
            }
            NormalizationBase::Inherited {
                schemas,
                presets,
                root_replaced,
            } => (schemas, presets, root_replaced),
        };
        let mut normalized = pending.schema;
        self.definition_stack.push(Rc::clone(&pending.definition));
        self.normalize_conditional_default_tree(
            InheritedDefaults {
                schemas: &bases,
                presets: &presets,
                root_replaced,
            },
            &mut normalized,
            expression,
            pending.definition.source(),
            &mut local_budget,
            depth + 1,
        )?;
        self.definition_stack.pop();
        budget.work = local_budget.work;
        budget.copied_nodes = local_budget.copied_nodes;
        budget.copied_bytes = local_budget.copied_bytes;
        self.normalized_defaults[ordinal] = Some(normalized);
        Ok(())
    }

    fn normalized_schema(
        &mut self,
        schema: &Json,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<Option<Json>, NativeError> {
        let Some(ordinal) = schema.get(PENDING_DEFAULTS).and_then(Json::as_u64) else {
            return Ok(None);
        };
        let ordinal = ordinal as usize;
        self.resolve_normalization(ordinal, budget, depth)?;
        let normalized = self.normalized_defaults[ordinal]
            .as_ref()
            .expect("deferred defaults have been resolved");
        budget.charge_copy(normalized)?;
        let mut normalized = normalized.clone();
        preserve_narrowing_annotations(schema, &mut normalized, budget)?;
        if let Some(next) = self.normalized_schema(&normalized, budget, depth + 1)? {
            normalized = next;
        }
        Ok(Some(normalized))
    }
    fn defer_inherited_normalization(
        &mut self,
        inherited: InheritedDefaults<'_>,
        schema: &mut Json,
        expression: &super::Expression,
        _source: &str,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        if !contains_union(expression) {
            return Ok(());
        }
        budget.charge(depth)?;
        let ordinal = self.pending_normalizations.len();
        if ordinal >= crate::schema::profile::MAX_SCHEMA_NODES {
            return Err(NativeError::new(
                NativeErrorKind::LimitExceeded,
                "native deferred defaults exceed schema-node bound",
            )
            .at_path(budget.file)
            .at_span(expression.span));
        }
        budget.charge_copy(schema)?;
        for base in inherited.schemas {
            budget.charge_copy(base)?;
        }
        for preset in inherited.presets {
            budget.charge_preset_copy(preset)?;
        }
        let id = normalization_id(ordinal);
        admit_copy_cost(
            &mut budget.copied_nodes,
            &mut budget.copied_bytes,
            3,
            PENDING_DEFAULTS.len() + id.len(),
            budget.file,
            Some(expression.span),
        )?;
        self.pending_normalizations.push(Some(PendingNormalization {
            schema: schema.clone(),
            bases: NormalizationBase::Inherited {
                schemas: inherited.schemas.to_vec(),
                presets: inherited.presets.to_vec(),
                root_replaced: inherited.root_replaced,
            },
            definition: Rc::clone(
                self.definition_stack
                    .last()
                    .expect("normalization owns its definition"),
            ),
            span: expression.span,
            file: budget.file.to_owned(),
        }));
        self.normalized_defaults.push(None);
        self.schema_sources
            .insert(id, SchemaSource::Normalization(ordinal));
        schema
            .as_object_mut()
            .expect("native schema is an object")
            .insert(PENDING_DEFAULTS.into(), Json::from(ordinal));
        Ok(())
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
        if let Some(normalized) = self.normalized_schema(local_schema, budget, depth)? {
            *local_schema = normalized;
        }
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
                    self.defer_inherited_normalization(
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
                            self.project_schema_defaults(schema, path, file, budget, depth + 1)?;
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
                    self.defer_inherited_normalization(
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
                        self.defer_inherited_normalization(
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
                self.defer_inherited_normalization(
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
                    self.defer_inherited_normalization(
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
            K::Literal(_) | K::Regex { .. } => {}
            K::Intersection(items) => {
                if let Some(branches) = local_schema.get_mut("allOf").and_then(Json::as_array_mut) {
                    for (item, branch) in items.iter().zip(branches) {
                        self.defer_inherited_normalization(
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
        &mut self,
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
        &mut self,
        schema: &Json,
        key: &str,
        visited: &mut HashSet<String>,
        found: &mut Vec<Json>,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        budget.charge(depth)?;
        let normalized = self.normalized_schema(schema, budget, depth)?;
        let schema = normalized.as_ref().unwrap_or(schema);
        if let Some(object) = schema.as_object() {
            if let Some(property) = crate::schema::profile::property(schema, key) {
                budget.charge_copy(property)?;
                found.push(property.clone());
            }
            if let Some(reference) = object.get("$ref").and_then(Json::as_str)
                && visited.insert(reference.to_owned())
            {
                let target = self.schema_for_reference(reference, budget, depth + 1)?;
                self.collect_property_schemas(&target, key, visited, found, budget, depth + 1)?;
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
    fn default_reference(
        &self,
        id: usize,
        budget: &mut WalkBudget<'_>,
    ) -> Result<Json, NativeError> {
        let reference = &self.default_positions[id].reference;
        admit_copy_cost(
            &mut budget.copied_nodes,
            &mut budget.copied_bytes,
            2,
            "$ref".len() + reference.len(),
            budget.file,
            Some(budget.span),
        )?;
        Ok(Json::Object(Map::from_iter([(
            "$ref".into(),
            Json::String(reference.clone()),
        )])))
    }

    fn materialize_default_constraints(
        &self,
        state: &DefaultMemoState,
        fixed: &[(usize, usize)],
        origin: usize,
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<Json, NativeError> {
        if state.is_empty() {
            return Err(default_invariant(
                budget,
                "a default check has no constraint alternatives",
            ));
        }
        let origin_is_fixed = fixed
            .binary_search_by_key(&origin, |choice| choice.0)
            .is_ok();
        let common_count = fixed.len() + usize::from(!origin_is_fixed);
        for term in state {
            for id in fixed
                .iter()
                .map(|choice| choice.0)
                .chain((!origin_is_fixed).then_some(origin))
            {
                budget.work = budget
                    .work
                    .saturating_add(term.len().max(1).ilog2() as usize + 1);
                budget.charge(depth)?;
                if term.binary_search(&id).is_err() {
                    return Err(default_invariant(
                        budget,
                        "a required source is not common to every supporting term",
                    ));
                }
            }
        }
        let mut alternatives = Vec::new();
        for term in state {
            admit_copy_cost(
                &mut budget.copied_nodes,
                &mut budget.copied_bytes,
                2,
                term.len() * std::mem::size_of::<Json>() + "allOf".len(),
                budget.file,
                Some(budget.span),
            )?;
            let mut conjuncts = Vec::with_capacity(term.len());
            for id in term {
                budget.charge(depth)?;
                conjuncts.push(self.default_reference(*id, budget)?);
            }
            alternatives.push(match conjuncts.len() {
                0 => {
                    return Err(default_invariant(
                        budget,
                        "a candidate constraint term is empty",
                    ));
                }
                1 => conjuncts.pop().expect("one default constraint"),
                _ => Json::Object(Map::from_iter([("allOf".into(), Json::Array(conjuncts))])),
            });
        }
        admit_copy_cost(
            &mut budget.copied_nodes,
            &mut budget.copied_bytes,
            2,
            "anyOf".len(),
            budget.file,
            Some(budget.span),
        )?;
        let multiple = alternatives.len() > 1;
        let schema = if multiple {
            Json::Object(Map::from_iter([(
                "anyOf".into(),
                Json::Array(alternatives),
            )]))
        } else {
            alternatives
                .pop()
                .expect("a supported default has a constraint term")
        };
        if !multiple {
            return Ok(schema);
        }
        admit_copy_cost(
            &mut budget.copied_nodes,
            &mut budget.copied_bytes,
            2,
            (common_count + 1) * std::mem::size_of::<Json>() + "allOf".len(),
            budget.file,
            Some(budget.span),
        )?;
        let mut conjuncts = Vec::with_capacity(common_count + 1);
        conjuncts.push(schema);
        // Every term already contains these exact original operations. Making
        // them directly reachable exposes no-work pins and the suppressed
        // candidate origin; Core deduplicates the repeated target SchemaIds.
        for id in fixed
            .iter()
            .map(|choice| choice.0)
            .chain((!origin_is_fixed).then_some(origin))
        {
            conjuncts.push(self.default_reference(id, budget)?);
        }
        Ok(Json::Object(Map::from_iter([(
            "allOf".into(),
            Json::Array(conjuncts),
        )])))
    }

    fn record_default_check(
        &mut self,
        constraints: &DefaultMemoState,
        origin: usize,
        fixed: &[(usize, usize)],
        budget: &mut WalkBudget<'_>,
        depth: usize,
    ) -> Result<(), NativeError> {
        if self.default_schema(origin).get("default").is_none() {
            return Err(default_invariant(
                budget,
                "a descriptor does not own a literal default",
            ));
        }
        let mut schema =
            self.materialize_default_constraints(constraints, fixed, origin, budget, depth)?;
        let ordinal = self.checks.len();
        let name = format!("native_default_check_{ordinal}");
        let id = format!("urn:daml:native:default-check:{ordinal}");
        let reference = &self.default_positions[origin].reference;
        admit_copy_cost(
            &mut budget.copied_nodes,
            &mut budget.copied_bytes,
            5,
            name.len() + id.len() + reference.len() + budget.file.as_os_str().len(),
            budget.file,
            Some(budget.span),
        )?;
        let candidate_origin = reference.clone();
        let mut fixed_choices = Vec::new();
        for (position, branch) in fixed {
            let reference = &self.default_positions[*position].reference;
            admit_copy_cost(
                &mut budget.copied_nodes,
                &mut budget.copied_bytes,
                1,
                reference.len() + std::mem::size_of::<usize>(),
                budget.file,
                Some(budget.span),
            )?;
            fixed_choices.push((reference.clone(), *branch));
        }
        schema
            .as_object_mut()
            .expect("generated default check is an object")
            .insert("$id".into(), Json::String(id));
        self.check_types.insert(
            name.clone(),
            Json::Object(Map::from_iter([("schema".into(), schema)])),
        );
        self.checks.push(DefaultCheck {
            schema_type: name,
            path: budget.file.to_owned(),
            span: budget.span,
            candidate_origin,
            fixed_choices,
        });
        Ok(())
    }
}

fn schema_has_default(
    schema: &Json,
    budget: &mut WalkBudget<'_>,
    depth: usize,
) -> Result<bool, NativeError> {
    budget.charge(depth)?;
    if schema.get("default").is_some() {
        return Ok(true);
    }
    for child in crate::schema::profile::children(schema) {
        if schema_has_default(child.value, budget, depth + 1)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn copy_default_monomial(
    monomial: &[usize],
    budget: &mut WalkBudget<'_>,
    depth: usize,
) -> Result<DefaultMonomial, NativeError> {
    budget.work = budget.work.saturating_add(monomial.len());
    budget.charge(depth)?;
    admit_copy_cost(
        &mut budget.copied_nodes,
        &mut budget.copied_bytes,
        monomial.len() + 1,
        std::mem::size_of_val(monomial),
        budget.file,
        Some(budget.span),
    )?;
    Ok(monomial.to_vec())
}

fn merge_default_monomials(
    left: &[usize],
    right: &[usize],
    budget: &mut WalkBudget<'_>,
    depth: usize,
) -> Result<DefaultMonomial, NativeError> {
    if right.is_empty() {
        return copy_default_monomial(left, budget, depth);
    }
    let length = left.len() + right.len();
    budget.work = budget.work.saturating_add(2 * length);
    budget.charge(depth)?;
    admit_copy_cost(
        &mut budget.copied_nodes,
        &mut budget.copied_bytes,
        length + 1,
        length * std::mem::size_of::<usize>(),
        budget.file,
        Some(budget.span),
    )?;
    let mut merged = Vec::with_capacity(length);
    let (mut l, mut r) = (0, 0);
    while l < left.len() && r < right.len() {
        match left[l].cmp(&right[r]) {
            std::cmp::Ordering::Less => {
                merged.push(left[l]);
                l += 1;
            }
            std::cmp::Ordering::Greater => {
                merged.push(right[r]);
                r += 1;
            }
            std::cmp::Ordering::Equal => {
                merged.push(left[l]);
                l += 1;
                r += 1;
            }
        }
    }
    merged.extend_from_slice(&left[l..]);
    merged.extend_from_slice(&right[r..]);
    Ok(merged)
}

fn default_invariant(budget: &WalkBudget<'_>, message: &str) -> NativeError {
    NativeError::new(
        NativeErrorKind::Schema,
        format!("native default analysis invariant failed: {message}"),
    )
    .at_path(budget.file)
    .at_span(budget.span)
}

fn supporting_default_constraints(
    constraints: &DefaultMemoState,
    source: &[usize],
    budget: &mut WalkBudget<'_>,
    depth: usize,
) -> Result<DefaultMemoState, NativeError> {
    let mut supported = Vec::new();
    for term in constraints {
        let width = term.len().max(1).ilog2() as usize + 1;
        budget.work = budget
            .work
            .saturating_add(source.len().saturating_mul(width));
        budget.charge(depth)?;
        if source.iter().all(|id| term.binary_search(id).is_ok()) {
            charge_default_monomial(term.len(), budget, depth)?;
            supported.push(term.clone());
        }
    }
    Ok(supported)
}

fn default_memo_product(
    state: &DefaultMemoState,
    factor: &DefaultMemoState,
    budget: &mut WalkBudget<'_>,
    depth: usize,
) -> Result<DefaultMemoState, NativeError> {
    let mut product = Vec::new();
    for left in state {
        for right in factor {
            product.push(merge_default_monomials(left, right, budget, depth)?);
        }
    }
    canonical_default_monomials(&mut product, budget, depth)?;
    Ok(product)
}

fn canonical_default_monomials(
    state: &mut DefaultMemoState,
    budget: &mut WalkBudget<'_>,
    depth: usize,
) -> Result<(), NativeError> {
    let width = state.len().max(1).ilog2() as usize + 1;
    let atoms = state.iter().fold(0usize, |count, monomial| {
        count.saturating_add(monomial.len().saturating_add(1))
    });
    budget.work = budget.work.saturating_add(atoms.saturating_mul(width));
    budget.charge(depth)?;
    state.sort_unstable();
    state.dedup();
    Ok(())
}

fn charge_default_monomial(
    atoms: usize,
    budget: &mut WalkBudget<'_>,
    depth: usize,
) -> Result<(), NativeError> {
    let width = atoms.max(1).ilog2() as usize + 1;
    budget.work = budget.work.saturating_add(atoms.saturating_mul(width));
    budget.charge(depth)?;
    admit_copy_cost(
        &mut budget.copied_nodes,
        &mut budget.copied_bytes,
        atoms.saturating_add(1),
        atoms.saturating_mul(std::mem::size_of::<usize>()),
        budget.file,
        Some(budget.span),
    )
}

fn contains_union(expression: &super::Expression) -> bool {
    use super::ExpressionKind as K;
    match &expression.kind {
        K::Union(_) => true,
        K::Tagged {
            body: Some(body), ..
        }
        | K::Group(body)
        | K::Closed(body) => contains_union(body),
        K::Record(members) => members
            .iter()
            .any(|member| contains_union(&member.constraint)),
        K::Tuple(items) | K::Intersection(items) => items.iter().any(contains_union),
        _ => false,
    }
}

fn expression_at(expression: &super::Expression, span: Span) -> Option<&super::Expression> {
    if expression.span == span {
        return Some(expression);
    }
    use super::ExpressionKind as K;
    match &expression.kind {
        K::Tagged {
            body: Some(body), ..
        }
        | K::Group(body)
        | K::Closed(body) => expression_at(body, span),
        K::Record(members) => members
            .iter()
            .find_map(|member| expression_at(&member.constraint, span)),
        K::Tuple(items) | K::Union(items) | K::Intersection(items) => {
            items.iter().find_map(|item| expression_at(item, span))
        }
        _ => None,
    }
}

fn normalization_id(ordinal: usize) -> String {
    format!("urn:daml:native:default-normalization:{ordinal}")
}

fn preserve_narrowing_annotations(
    source: &Json,
    target: &mut Json,
    budget: &mut WalkBudget<'_>,
) -> Result<(), NativeError> {
    for keyword in [
        "$id",
        "default",
        "x-daml-default-tags",
        "x-daml-default-temporals",
        "x-daml-native-optional",
    ] {
        if let Some(value) = source.get(keyword)
            && target.get(keyword) != Some(value)
        {
            budget.charge_copy(value)?;
            target
                .as_object_mut()
                .expect("native schema is an object")
                .insert(keyword.into(), value.clone());
        }
    }
    Ok(())
}

fn materialize_normalizations(
    schema: &mut Json,
    budget: &mut WalkBudget<'_>,
    depth: usize,
) -> Result<(), NativeError> {
    budget.charge(depth)?;
    if let Some(ordinal) = schema.get(PENDING_DEFAULTS).and_then(Json::as_u64) {
        let mut reference = Json::Object(Map::from_iter([(
            "$ref".into(),
            Json::String(normalization_id(ordinal as usize)),
        )]));
        budget.charge_copy(&reference)?;
        preserve_narrowing_annotations(schema, &mut reference, budget)?;
        *schema = reference;
        return Ok(());
    }
    let mut error = None;
    crate::schema::profile::for_each_child_mut(schema, |child| {
        if error.is_none() {
            error = materialize_normalizations(child, budget, depth + 1).err();
        }
    });
    error.map_or(Ok(()), Err)
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
        K::Literal(_) | K::Regex { .. } | K::Union(_) | K::Intersection(_) => false,
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
        K::Literal(_) | K::Regex { .. } => {}
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
        K::Literal(_) | K::Regex { .. } => {}
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
        "Date" | "Time" | "DateTime" | "Cron" => {
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
        K::Literal(_) | K::Regex { .. } | K::Tagged { .. } | K::Record(_) | K::Tuple(_) => false,
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
        K::Literal(_) | K::Regex { .. } | K::Tagged { .. } | K::Record(_) | K::Tuple(_) => None,
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
    value: &'a Json,
    schema: crate::schema::SchemaView<'a>,
    location: &'a crate::SchemaLocation,
) -> crate::ResolvedNode<'a> {
    use crate::resolved::ResolvedTag;
    use crate::{Origin, ResolvedNode, ResolvedValue};
    use std::{borrow::Cow, collections::BTreeMap};
    let origin = Origin::Default(location);
    // Registry admission owns tag and temporal identity, including descendants.
    // Reconstruct the candidate before Core suppresses its own root payload.
    let tag = schema
        .default_tag(value)
        .map(|(name, binding)| ResolvedTag {
            name: Cow::Owned(name.to_owned()),
            binding: binding.map(|binding| Cow::Owned(binding.to_owned())),
            origin,
            supplied: false,
        });
    let resolved = match value {
        Json::Null => ResolvedValue::Null,
        Json::Bool(value) => ResolvedValue::Bool(*value),
        Json::Number(value) => ResolvedValue::Number(Cow::Owned(value.clone())),
        Json::String(text) => match schema.default_temporal(value) {
            Some(value) => ResolvedValue::Temporal(value.into_owned()),
            None => ResolvedValue::String(Cow::Owned(text.clone())),
        },
        Json::Array(items) => ResolvedValue::Sequence(
            items
                .iter()
                .map(|item| resolved_node(item, schema, location))
                .collect(),
        ),
        Json::Object(fields) => {
            let mut resolved = BTreeMap::new();
            for (key, item) in fields {
                resolved.insert(
                    Cow::Owned(key.clone()),
                    resolved_node(item, schema, location),
                );
            }
            ResolvedValue::Mapping(resolved)
        }
    };
    ResolvedNode {
        value: resolved,
        tag,
        origin,
        opaque: false,
        number_class: 0,
    }
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
