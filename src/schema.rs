use crate::{
    json_backend::{DamlJson, NodeRef},
    resolved::{ResolvedNode, SchemaLocation},
    schema_error::{SchemaError, SchemaErrorKind},
};
use jsonschema::{
    Draft, Keyword, PatternOptions, Registry, Resource, ValidationError, Validator,
    error::ValidationErrorKind, json::Node as _,
};
use regex_automata::{Anchored, Input, MatchKind, meta::Regex};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

pub(crate) mod profile;
use profile::{MAX_COMPILE_NODES, MAX_SCHEMA_DEPTH, MAX_SCHEMA_NODES, MAX_SOURCE_BYTES};

const REGEX_SIZE_LIMIT: usize = 1 << 20;
const NATIVE_REGEX_KEYWORD: &str = "x-daml-regex";

pub(crate) fn native_regex_origin(origin: u64) -> String {
    format!("urn:daml:native:regex-origin:{origin}")
}

struct NativeRegex(Regex);

impl<'i> Keyword<'i, DamlJson> for NativeRegex {
    fn validate(&self, instance: NodeRef<'i>) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::custom(
                "string does not match the required Regex",
            ))
        }
    }

    fn is_valid(&self, instance: NodeRef<'i>) -> bool {
        instance.as_string().is_none_or(|value| {
            self.0
                .find(Input::new(value.as_ref()).anchored(Anchored::Yes))
                .is_some_and(|found| found.end() == value.len())
        })
    }
}

fn native_regex_factory<'a>(
    _parent: &'a Map<String, Value>,
    value: &'a Value,
    _path: jsonschema::paths::Location,
) -> Result<Box<dyn for<'i> Keyword<'i, DamlJson>>, ValidationError<'a>> {
    let config = value
        .as_object()
        .ok_or_else(|| ValidationError::schema("DAML Regex metadata must be an object"))?;
    let pattern = config
        .get("pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| ValidationError::schema("DAML Regex source must be a string"))?;
    config
        .get("origin")
        .and_then(Value::as_u64)
        .ok_or_else(|| ValidationError::schema("DAML Regex origin must be an integer"))?;
    let regex = Regex::builder()
        .configure(
            Regex::config()
                .match_kind(MatchKind::All)
                .nfa_size_limit(Some(REGEX_SIZE_LIMIT))
                .onepass_size_limit(Some(REGEX_SIZE_LIMIT))
                .dfa_size_limit(Some(REGEX_SIZE_LIMIT)),
        )
        .build(pattern)
        .map_err(|error| {
            let message = std::error::Error::source(&error)
                .map_or_else(|| error.to_string(), |source| format!("{error}: {source}"));
            ValidationError::schema(format!("invalid DAML Regex: {message}"))
        })?;
    Ok(Box::new(NativeRegex(regex)))
}

#[derive(Clone)]
struct Binding {
    value: Value,
    id: String,
}

/// A parsed schema module. Compose modules explicitly to create a registry.
#[derive(Clone, Default)]
pub struct SchemaModule {
    types: HashMap<String, Binding>,
    resources: HashMap<String, Value>,
}

impl SchemaModule {
    /// Loads a JSON-compatible module document.
    pub fn from_json(source: &str) -> Result<Self, SchemaError> {
        check_source(source)?;
        let value = crate::strict_json::parse(source)?;
        Self::from_value(value)
    }

    /// Loads a module written in DAML's explicit-value syntax.
    pub fn from_daml(source: &str) -> Result<Self, SchemaError> {
        check_source(source)?;
        let document = crate::parse(source).map_err(|error| {
            SchemaError::new(
                SchemaErrorKind::Parse,
                format!("invalid schema DAML: {error}"),
            )
            .caused_by(error)
        })?;
        check_daml_limits(&document, document.root())?;
        Self::from_value(daml_value(&document, document.root())?)
    }

    /// Loads a module from an already constructed JSON-compatible value.
    ///
    /// This is the interchange boundary for consumer-owned schema adapters.
    /// Translate foreign schemas into the native module format before calling;
    /// DAML does not interpret application-specific wire fields or references.
    ///
    /// # Errors
    ///
    /// Rejects malformed declarations and values exceeding schema bounds.
    pub fn from_value(value: Value) -> Result<Self, SchemaError> {
        check_value_limits(&value)?;
        let root = object(&value, "module")?;
        reject_unknown(root, &["types", "resources"], "module")?;
        let mut module = Self::default();
        if let Some(types) = root.get("types") {
            for (name, item) in object(types, "types")? {
                valid_name(name)?;
                let item = object(item, "type binding")?;
                reject_unknown(item, &["schema"], "type binding")?;
                let schema = item
                    .get("schema")
                    .ok_or_else(|| invalid("type binding requires schema"))?;
                check_schema_shape(schema)?;
                let id = schema_id(schema).unwrap_or_else(|| generated_id(name));
                let mut value = schema.clone();
                assign_id(&mut value, &id)?;
                module.types.insert(name.clone(), Binding { value, id });
            }
        }
        if let Some(resources) = root.get("resources") {
            for item in resources
                .as_array()
                .ok_or_else(|| invalid("resources must be an array"))?
            {
                let item = object(item, "resource")?;
                reject_unknown(item, &["uri", "schema"], "resource")?;
                let uri = item
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("resource requires string uri"))?;
                let schema = item
                    .get("schema")
                    .ok_or_else(|| invalid("resource requires schema"))?;
                check_schema_shape(schema)?;
                if module
                    .resources
                    .insert(uri.to_owned(), schema.clone())
                    .is_some()
                {
                    return Err(duplicate(format!("duplicate resource URI {uri:?}")));
                }
            }
        }
        Ok(module)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SchemaId(usize);

#[derive(Clone, Copy)]
pub(crate) struct SchemaView<'a> {
    registry: &'a Schemas,
    id: SchemaId,
}

impl<'a> SchemaView<'a> {
    fn entry(self) -> &'a SchemaEntry {
        &self.registry.entries[self.id.0]
    }
    pub(crate) fn id(self) -> SchemaId {
        self.id
    }
    pub(crate) fn value(self) -> &'a Value {
        &self.entry().value
    }
    pub(crate) fn location(self) -> &'a SchemaLocation {
        &self.entry().location
    }
    pub(crate) fn default_origin(self) -> Option<&'a SchemaLocation> {
        self.entry().default_origin.as_ref()
    }
    pub(crate) fn default_tag(&self, value: &Value) -> Option<(&'a str, Option<&'a str>)> {
        let entry = self.entry();
        let index = *entry.default_tags.get(&(value as *const Value as usize))?;
        let annotation = &entry.value["x-daml-default-tags"][index];
        Some((
            annotation["tag"].as_str().expect("validated tag"),
            annotation.get("binding").and_then(Value::as_str),
        ))
    }
    pub(crate) fn referenced(self) -> Option<Self> {
        self.registry
            .references
            .get(&self.id)
            .copied()
            .map(|id| self.registry.view(id))
    }
    pub(crate) fn property(self, key: &str) -> Option<Self> {
        self.registry.locate(profile::property(self.value(), key)?)
    }
    pub(crate) fn requires_property(self, key: &str) -> bool {
        self.value()
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| required.iter().any(|name| name.as_str() == Some(key)))
    }

    pub(crate) fn container_kind(self) -> Option<&'static str> {
        match self.value().get("type")?.as_str()? {
            "object" => Some("object"),
            "array" => Some("array"),
            _ => None,
        }
    }
    pub(crate) fn property_names(self) -> impl Iterator<Item = &'a str> {
        self.value()
            .get("properties")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|properties| properties.keys().map(String::as_str))
    }
    pub(crate) fn item(self, index: usize) -> Option<Self> {
        self.registry.locate(profile::item(self.value(), index)?)
    }
    pub(crate) fn array_child(self, keyword: &str, index: usize) -> Option<Self> {
        // Index before resolving: mapped Iterator::nth also resolves skipped children.
        let value = self.value().get(keyword)?.as_array()?.get(index)?;
        self.registry.locate(value)
    }
    pub(crate) fn array_children(
        self,
        keyword: &str,
    ) -> impl ExactSizeIterator<Item = Self> + use<'a> {
        self.value()
            .get(keyword)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(move |value| {
                self.registry
                    .locate(value)
                    .expect("preflight indexed every child schema")
            })
    }
    pub(crate) fn has_defaults(self) -> bool {
        self.entry().has_defaults
    }
    pub(crate) fn uses_uniqueness(self) -> bool {
        self.entry().uses_uniqueness
    }
    pub(crate) fn evaluation_cost(self, units: usize) -> usize {
        self.entry().work.saturating_mul(units)
    }
    pub(crate) fn validate(self, node: &ResolvedNode<'_>) -> Result<(), SchemaFailure> {
        let entry = self.entry();
        entry
            .validator
            .validate(NodeRef::new(node))
            .map_err(|error| {
                let schema_location = validation_error_location(&error, &entry.location);
                SchemaFailure {
                    path: error.instance_path().to_string(),
                    schema: schema_location,
                    message: error.masked().to_string(),
                }
            })
    }
}
struct SchemaEntry {
    value: Value,
    location: SchemaLocation,
    default_origin: Option<SchemaLocation>,
    default_tags: HashMap<usize, usize>,
    validator: Validator<DamlJson>,
    work: usize,
    has_defaults: bool,
    uses_uniqueness: bool,
}

/// Immutable, explicitly composed schema registry and its compiled validators.
pub struct Schemas {
    entries: Vec<SchemaEntry>,
    by_type: HashMap<String, SchemaId>,
    by_address: HashMap<usize, SchemaId>,
    references: HashMap<SchemaId, SchemaId>,
    source_tags: Option<HashSet<String>>,
}

impl Schemas {
    /// Composes modules without last-writer-wins behavior.
    pub fn compose(modules: impl IntoIterator<Item = SchemaModule>) -> Result<Self, SchemaError> {
        let mut types = HashMap::<String, Binding>::new();
        let mut resources = HashMap::<String, Value>::new();
        let (mut input_nodes, mut input_bytes) = (0usize, 0usize);
        for module in modules {
            for (name, binding) in &module.types {
                input_nodes = input_nodes.saturating_add(1);
                input_bytes = input_bytes
                    .saturating_add(name.len())
                    .saturating_add(binding.id.len());
                let (nodes, bytes) = check_value_limits(&binding.value)?;
                input_nodes = input_nodes.saturating_add(nodes);
                input_bytes = input_bytes.saturating_add(bytes);
            }
            for (uri, schema) in &module.resources {
                let (nodes, bytes) = check_value_limits(schema)?;
                input_nodes = input_nodes.saturating_add(nodes);
                input_bytes = input_bytes.saturating_add(bytes).saturating_add(uri.len());
            }
            if input_nodes > MAX_SCHEMA_NODES || input_bytes > MAX_SOURCE_BYTES {
                return Err(limit("composed modules exceed aggregate node/byte bounds"));
            }
            for (name, value) in module.types {
                if types.insert(name.clone(), value).is_some() {
                    return Err(duplicate(format!("duplicate type {name:?}")));
                }
            }
            for (uri, value) in module.resources {
                if resources.insert(uri.clone(), value).is_some() {
                    return Err(duplicate(format!("duplicate resource URI {uri:?}")));
                }
            }
        }
        let registered = types.keys().map(String::as_str).collect::<HashSet<_>>();
        for schema in types
            .values()
            .map(|binding| &binding.value)
            .chain(resources.values())
        {
            validate_default_bindings(schema, &registered)?;
        }
        let mut identifiers = HashSet::new();
        for binding in types.values() {
            if !identifiers.insert(binding.id.clone()) {
                return Err(duplicate(format!(
                    "duplicate schema identifier {:?}",
                    binding.id
                )));
            }
        }
        for uri in resources.keys() {
            if identifiers.contains(uri) {
                return Err(duplicate(format!("duplicate schema identifier {uri:?}")));
            }
        }
        drop(identifiers);
        let mut registry_contents = resources.clone();
        for binding in types.values() {
            registry_contents
                .entry(binding.id.clone())
                .or_insert_with(|| binding.value.clone());
        }
        validate_nested_identifiers(&registry_contents)?;
        for schema in types.values().map(|b| &b.value).chain(resources.values()) {
            validate_exact_numbers(schema)?;
        }
        let mut registry = Registry::new();
        for (uri, schema) in &registry_contents {
            registry = registry
                .add(uri, Resource::from_contents(schema.clone()))
                .map_err(|error| {
                    SchemaError::new(
                        SchemaErrorKind::InvalidSchema,
                        format!("invalid resource URI {uri:?}: {error}"),
                    )
                    .caused_by(error)
                })?;
        }
        let registry = registry.prepare().map_err(|error| {
            let kind = match &error {
                jsonschema::ReferencingError::Unretrievable { .. }
                | jsonschema::ReferencingError::PointerToNowhere { .. }
                | jsonschema::ReferencingError::NoSuchAnchor { .. } => {
                    SchemaErrorKind::UnknownReference
                }
                _ => SchemaErrorKind::InvalidSchema,
            };
            SchemaError::new(kind, format!("cannot prepare schema registry: {error}"))
                .caused_by(error)
        })?;
        let stats = prepare_schema_graph(&registry_contents, &registry)?;
        let options = jsonschema::options_for::<DamlJson>()
            .with_draft(Draft::Draft202012)
            .with_registry(&registry)
            .offline()
            .should_validate_formats(false)
            .with_keyword(NATIVE_REGEX_KEYWORD, native_regex_factory)
            .with_pattern_options(
                PatternOptions::regex()
                    .size_limit(REGEX_SIZE_LIMIT)
                    .dfa_size_limit(REGEX_SIZE_LIMIT),
            );
        let mut entries = Vec::new();
        let mut by_type = HashMap::new();
        let mut total = 0usize;
        for (name, binding) in types {
            let index = entries.len();
            append_schema_entries(
                binding.value,
                &binding.id,
                &options,
                &mut entries,
                &mut total,
            )?;
            by_type.insert(name, SchemaId(index));
        }
        for (uri, value) in resources {
            append_schema_entries(value, &uri, &options, &mut entries, &mut total)?;
        }
        let lookup = entries
            .iter()
            .enumerate()
            .map(|(id, e)| {
                (
                    (e.location.resource.clone(), e.location.pointer.clone()),
                    SchemaId(id),
                )
            })
            .collect::<HashMap<_, _>>();
        for entry in &mut entries {
            let info = stats
                .get(&entry.location)
                .expect("every compiled view was preflighted");
            entry.work = info.work;
            entry.has_defaults = info.defaults;
            entry.uses_uniqueness = info.uniqueness;
            if let Some(annotations) = entry
                .value
                .get("x-daml-default-tags")
                .and_then(Value::as_array)
            {
                let default = entry
                    .value
                    .get("default")
                    .expect("tagged defaults are validated during admission");
                for (index, annotation) in annotations.iter().enumerate() {
                    let path = annotation["path"]
                        .as_str()
                        .expect("annotation paths are validated");
                    let target = default
                        .pointer(path)
                        .expect("annotation paths target a default value");
                    entry
                        .default_tags
                        .insert(target as *const Value as usize, index);
                }
            }
        }
        let mut prepared_addresses = HashMap::new();
        for (id, entry) in entries.iter().enumerate() {
            let uri = jsonschema::uri::from_str(&entry.location.resource).map_err(|error| {
                invalid(format!(
                    "invalid schema URI {:?}: {error}",
                    entry.location.resource
                ))
            })?;
            let pointer_reference =
                canonical_reference(&entry.location.resource, &entry.location.pointer);
            let target = registry
                .resolver(uri)
                .lookup(&pointer_reference)
                .map_err(|error| {
                    unknown(format!(
                        "cannot locate schema at {}#{}: {error}",
                        entry.location.resource, entry.location.pointer
                    ))
                })?;
            prepared_addresses.insert(target.contents() as *const Value as usize, SchemaId(id));
        }
        let mut by_address = HashMap::new();
        for (id, entry) in entries.iter().enumerate() {
            index_nested_addresses(
                &entry.value,
                &entry.location.resource,
                &entry.location.pointer,
                SchemaId(id),
                &lookup,
                &mut by_address,
            );
        }
        let mut references = HashMap::new();
        for (id, entry) in entries.iter().enumerate() {
            let Some(reference) = entry.value.get("$ref").and_then(Value::as_str) else {
                continue;
            };
            let uri = jsonschema::uri::from_str(&entry.location.resource).map_err(|error| {
                invalid(format!(
                    "invalid schema URI {:?}: {error}",
                    entry.location.resource
                ))
            })?;
            let pointer_reference =
                canonical_reference(&entry.location.resource, &entry.location.pointer);
            let source = registry
                .resolver(uri)
                .lookup(&pointer_reference)
                .map_err(|error| {
                    unknown(format!(
                        "cannot locate schema at {}#{}: {error}",
                        entry.location.resource, entry.location.pointer
                    ))
                })?;
            let target = source.resolver().lookup(reference).map_err(|error| {
                unknown(format!(
                    "unresolved schema reference {reference:?}: {error}"
                ))
            })?;
            let target_id = prepared_addresses
                .get(&(target.contents() as *const Value as usize))
                .copied()
                .expect("preflight admitted only references to indexed schema nodes");
            references.insert(SchemaId(id), target_id);
        }
        Ok(Self {
            entries,
            by_type,
            by_address,
            references,
            source_tags: None,
        })
    }

    /// Restricts source spellings available to semantic serialization.
    ///
    /// Native compilation supplies explicit registered source names; low-level
    /// composed modules retain their default visibility unless this is called.
    pub(crate) fn restrict_source_tags(&mut self, tags: impl IntoIterator<Item = String>) {
        self.source_tags = Some(tags.into_iter().collect());
    }

    /// Serializes semantic data while preserving asserted schema binding identity.
    pub fn format_semantic(
        &self,
        node: &crate::SemanticNode,
        options: crate::FormatOptions,
    ) -> Result<String, crate::SemanticError> {
        crate::semantic::format_with_tags(node, options, |tag, path| {
            let visible = |name: &str| {
                self.source_tags
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(name))
            };
            match tag.binding.as_deref() {
                Some(binding) => {
                    let Some(binding_id) = self.binding_identity(binding) else {
                        return Err(crate::semantic::unrepresentable_tag(path));
                    };
                    if tag.name.len() <= crate::semantic::MAX_SCALAR_BYTES
                        && visible(&tag.name)
                        && self.binding_identity(&tag.name) == Some(binding_id)
                    {
                        return Ok(tag.name.clone());
                    }
                    self.by_type
                        .keys()
                        .filter(|name| {
                            name.len() <= crate::semantic::MAX_SCALAR_BYTES
                                && visible(name)
                                && self.binding_identity(name) == Some(binding_id)
                        })
                        .min()
                        .cloned()
                        .ok_or_else(|| crate::semantic::unrepresentable_tag(path))
                }
                None => {
                    if tag.name.len() > crate::semantic::MAX_SCALAR_BYTES {
                        Err(crate::semantic::tag_limit_error(path))
                    } else if crate::names::valid_tag(&tag.name) && visible(&tag.name) {
                        Ok(tag.name.clone())
                    } else {
                        Err(crate::semantic::unrepresentable_tag(path))
                    }
                }
            }
        })
    }
    fn binding_identity(&self, name: &str) -> Option<SchemaId> {
        let mut current = self.by_type.get(name).copied().or_else(|| {
            self.entries.iter().enumerate().find_map(|(index, entry)| {
                (entry.location.pointer.is_empty() && entry.location.resource == name)
                    .then_some(SchemaId(index))
            })
        })?;
        let mut steps = 0;
        while let Some(next) = self.references.get(&current).copied() {
            let entry = &self.entries[current.0];
            let pure_alias = entry.value.as_object().is_some_and(|object| {
                object.get("$ref").and_then(Value::as_str).is_some()
                    && object.keys().all(|key| key == "$ref" || key == "$id")
                    && (object.len() == 1
                        || (object.len() == 2
                            && object.get("$id").and_then(Value::as_str)
                                == Some(entry.location.resource.as_str())))
            });
            if steps >= self.entries.len() {
                return None;
            }
            if !pure_alias {
                return Some(current);
            }
            current = next;
            steps += 1;
        }
        Some(current)
    }

    pub(crate) fn type_schema(&self, tag: &str) -> Option<SchemaView<'_>> {
        self.by_type.get(tag).copied().map(|id| self.view(id))
    }
    fn view(&self, id: SchemaId) -> SchemaView<'_> {
        SchemaView { registry: self, id }
    }
    fn locate(&self, value: &Value) -> Option<SchemaView<'_>> {
        self.by_address
            .get(&(value as *const Value as usize))
            .copied()
            .map(|id| self.view(id))
    }
}

pub(crate) struct SchemaFailure {
    pub path: String,
    pub schema: SchemaLocation,
    pub message: String,
}
fn index_nested_addresses(
    value: &Value,
    resource: &str,
    pointer: &str,
    id: SchemaId,
    lookup: &HashMap<(String, String), SchemaId>,
    addresses: &mut HashMap<usize, SchemaId>,
) {
    addresses.insert(value as *const Value as usize, id);
    for child in profile::children(value) {
        let path = child.pointer(pointer);
        let child_id = lookup
            .get(&(resource.to_owned(), path.clone()))
            .copied()
            .expect("all schema children have compiled entries");
        index_nested_addresses(child.value, resource, &path, child_id, lookup, addresses);
    }
}

fn append_schema_entries(
    value: Value,
    resource: &str,
    options: &jsonschema::ValidationOptions<'_, std::sync::Arc<dyn jsonschema::Retrieve>, DamlJson>,
    entries: &mut Vec<SchemaEntry>,
    total: &mut usize,
) -> Result<(), SchemaError> {
    fn visit(
        value: &Value,
        resource: &str,
        pointer: &str,
        options: &jsonschema::ValidationOptions<
            '_,
            std::sync::Arc<dyn jsonschema::Retrieve>,
            DamlJson,
        >,
        entries: &mut Vec<SchemaEntry>,
        total: &mut usize,
    ) -> Result<(), SchemaError> {
        *total = total.saturating_add(count_nodes(value));
        if *total > MAX_COMPILE_NODES {
            return Err(limit("aggregate schema compilation work exceeds bound"));
        }
        let target = canonical_reference(resource, pointer);
        let wrapper = serde_json::json!({"$ref":target});
        let location = SchemaLocation {
            resource: resource.into(),
            pointer: pointer.into(),
        };
        let validator = options
            .build(&wrapper)
            .map_err(|error| compile_error(error, location.clone()))?;
        let default_origin = value.get("default").map(|_| SchemaLocation {
            resource: resource.into(),
            pointer: crate::pointer::join(pointer, "default"),
        });
        entries.push(SchemaEntry {
            value: value.clone(),
            location,
            default_origin,
            default_tags: HashMap::new(),
            validator,
            work: 0,
            has_defaults: false,
            uses_uniqueness: false,
        });
        for child in profile::children(value) {
            visit(
                child.value,
                resource,
                &child.pointer(pointer),
                options,
                entries,
                total,
            )?;
        }
        Ok(())
    }
    let mut nodes = 0;
    walk_schema(&value, 0, &mut nodes)?;
    visit(&value, resource, "", options, entries, total)
}
fn canonical_reference(resource: &str, pointer: &str) -> String {
    let mut fragment = String::with_capacity(pointer.len());
    for byte in pointer.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            fragment.push(byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(fragment, "%{byte:02X}");
        }
    }
    format!("{resource}#{fragment}")
}
fn count_nodes(value: &Value) -> usize {
    let mut count = 0;
    let _ = walk_schema(value, 0, &mut count);
    count
}
fn percent_decode(source: &str) -> Option<String> {
    let bytes = source.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3
        } else {
            decoded.push(bytes[i]);
            i += 1
        }
    }
    String::from_utf8(decoded).ok()
}
fn check_daml_limits(document: &crate::Document, root: &crate::Node) -> Result<(), SchemaError> {
    fn visit(
        document: &crate::Document,
        node: &crate::Node,
        depth: usize,
        nodes: &mut usize,
        bytes: &mut usize,
    ) -> Result<(), SchemaError> {
        use crate::Value as V;
        *nodes = nodes.saturating_add(1);
        if *nodes > MAX_SCHEMA_NODES || depth > MAX_SCHEMA_DEPTH {
            return Err(limit("schema module exceeds node/depth bound"));
        }
        match &node.value {
            V::String(_) => {
                *bytes = bytes.saturating_add(
                    document
                        .string(node)
                        .ok_or_else(|| invalid("invalid string span"))?
                        .len(),
                )
            }
            V::Sequence(values) => {
                for value in values {
                    visit(document, value, depth + 1, nodes, bytes)?
                }
            }
            V::Mapping(values) => {
                for entry in values {
                    *bytes = bytes.saturating_add(
                        document
                            .slice(entry.key)
                            .ok_or_else(|| invalid("invalid key span"))?
                            .len(),
                    );
                    visit(document, &entry.value, depth + 1, nodes, bytes)?;
                }
            }
            _ => {}
        }
        if *bytes > MAX_SOURCE_BYTES {
            return Err(limit("schema module exceeds 8 MiB"));
        }
        Ok(())
    }
    let (mut nodes, mut bytes) = (0, 0);
    visit(document, root, 0, &mut nodes, &mut bytes)
}
fn check_value_limits(value: &Value) -> Result<(usize, usize), SchemaError> {
    fn string_size(value: &str) -> usize {
        2usize.saturating_add(
            value
                .chars()
                .map(|character| match character {
                    '"' | '\\' | '\u{08}' | '\u{0c}' | '\n' | '\r' | '\t' => 2,
                    '\u{00}'..='\u{1f}' => 6,
                    _ => character.len_utf8(),
                })
                .sum::<usize>(),
        )
    }
    fn visit(
        value: &Value,
        depth: usize,
        nodes: &mut usize,
        bytes: &mut usize,
    ) -> Result<(), SchemaError> {
        *nodes = nodes.saturating_add(1);
        if *nodes > MAX_SCHEMA_NODES || depth > MAX_SCHEMA_DEPTH {
            return Err(limit("module exceeds node/depth bound"));
        }
        match value {
            Value::Null => *bytes = bytes.saturating_add(4),
            Value::Bool(true) => *bytes = bytes.saturating_add(4),
            Value::Bool(false) => *bytes = bytes.saturating_add(5),
            Value::Number(number) => *bytes = bytes.saturating_add(number.as_str().len()),
            Value::String(value) => *bytes = bytes.saturating_add(string_size(value)),
            Value::Array(values) => {
                *bytes = bytes.saturating_add(2 + values.len().saturating_sub(1));
                for value in values {
                    visit(value, depth + 1, nodes, bytes)?
                }
            }
            Value::Object(values) => {
                *bytes = bytes.saturating_add(2 + values.len().saturating_sub(1));
                for (key, value) in values {
                    *bytes = bytes.saturating_add(string_size(key) + 1);
                    visit(value, depth + 1, nodes, bytes)?;
                }
            }
        }
        if *bytes > MAX_SOURCE_BYTES {
            return Err(limit("schema module exceeds 8 MiB"));
        }
        Ok(())
    }
    let (mut nodes, mut bytes) = (0, 0);
    visit(value, 0, &mut nodes, &mut bytes)?;
    Ok((nodes, bytes))
}
fn check_source(s: &str) -> Result<(), SchemaError> {
    if s.len() > MAX_SOURCE_BYTES {
        Err(limit("schema source exceeds 8 MiB"))
    } else {
        Ok(())
    }
}
fn invalid(s: impl Into<String>) -> SchemaError {
    SchemaError::new(SchemaErrorKind::InvalidSchema, s)
}
fn duplicate(s: impl Into<String>) -> SchemaError {
    SchemaError::new(SchemaErrorKind::DuplicateName, s)
}
fn unknown(s: impl Into<String>) -> SchemaError {
    SchemaError::new(SchemaErrorKind::UnknownReference, s)
}
fn limit(s: impl Into<String>) -> SchemaError {
    SchemaError::new(SchemaErrorKind::LimitExceeded, s)
}
fn object<'a>(v: &'a Value, what: &str) -> Result<&'a Map<String, Value>, SchemaError> {
    v.as_object()
        .ok_or_else(|| invalid(format!("{what} must be an object")))
}
fn reject_unknown(m: &Map<String, Value>, allowed: &[&str], what: &str) -> Result<(), SchemaError> {
    if let Some(k) = m.keys().find(|k| !allowed.contains(&k.as_str())) {
        Err(invalid(format!("unknown {what} field {k:?}")))
    } else {
        Ok(())
    }
}
fn valid_name(s: &str) -> Result<(), SchemaError> {
    if s.trim().is_empty() {
        Err(invalid("names must be nonempty"))
    } else {
        Ok(())
    }
}
fn generated_id(name: &str) -> String {
    use std::fmt::Write;

    let prefix = "urn:daml:type:";
    let mut id = String::with_capacity(prefix.len() + name.len() * 2);
    id.push_str(prefix);
    for byte in name.bytes() {
        write!(&mut id, "{byte:02x}").expect("writing to a String cannot fail");
    }
    id
}
fn schema_id(v: &Value) -> Option<String> {
    v.get("$id")?.as_str().map(str::to_owned)
}
fn assign_id(v: &mut Value, id: &str) -> Result<(), SchemaError> {
    if let Some(m) = v.as_object_mut() {
        m.entry("$id").or_insert_with(|| Value::String(id.into()));
        Ok(())
    } else if v.is_boolean() {
        *v = serde_json::json!({"$id":id,"allOf":[v.clone()]});
        Ok(())
    } else {
        Err(invalid("schema must be an object or boolean"))
    }
}
fn validate_nested_identifiers(resources: &HashMap<String, Value>) -> Result<(), SchemaError> {
    fn insert(
        ids: &mut HashMap<String, SchemaLocation>,
        uri: String,
        location: &SchemaLocation,
    ) -> Result<(), SchemaError> {
        if let Some(previous) = ids.get(&uri) {
            if previous != location {
                return Err(duplicate(format!("duplicate schema identifier {uri:?}")));
            }
        } else {
            ids.insert(uri, location.clone());
        }
        Ok(())
    }
    fn visit(
        schema: &Value,
        base: &jsonschema::Uri<String>,
        location: &SchemaLocation,
        ids: &mut HashMap<String, SchemaLocation>,
        anchors: &mut HashSet<(String, String)>,
    ) -> Result<(), SchemaError> {
        let Some(map) = schema.as_object() else {
            return Ok(());
        };
        let scope = if let Some(id) = map.get("$id").and_then(Value::as_str) {
            let uri = jsonschema::uri::resolve_against(&base.borrow(), id)
                .map_err(|error| invalid(format!("invalid schema identifier {id:?}: {error}")))?;
            if uri.has_fragment() {
                return Err(invalid("$id must not contain a fragment"));
            }
            insert(ids, uri.as_str().to_owned(), location)?;
            uri
        } else {
            base.clone()
        };
        if let Some(anchor) = map.get("$anchor").and_then(Value::as_str)
            && !anchors.insert((scope.as_str().to_owned(), anchor.to_owned()))
        {
            return Err(duplicate(format!(
                "duplicate schema anchor {anchor:?} in {scope}"
            )));
        }
        for child in profile::children(schema) {
            visit(
                child.value,
                &scope,
                &SchemaLocation {
                    resource: location.resource.clone(),
                    pointer: child.pointer(&location.pointer),
                },
                ids,
                anchors,
            )?;
        }
        Ok(())
    }
    let mut ids = HashMap::new();
    let mut anchors = HashSet::new();
    for resource in resources.keys() {
        let uri = jsonschema::uri::from_str(resource)
            .map_err(|error| invalid(format!("invalid resource URI {resource:?}: {error}")))?;
        insert(
            &mut ids,
            uri.as_str().to_owned(),
            &SchemaLocation {
                resource: resource.clone(),
                pointer: String::new(),
            },
        )?;
    }
    for (resource, schema) in resources {
        let uri = jsonschema::uri::from_str(resource)
            .map_err(|error| invalid(format!("invalid resource URI {resource:?}: {error}")))?;
        visit(
            schema,
            &uri,
            &SchemaLocation {
                resource: resource.clone(),
                pointer: String::new(),
            },
            &mut ids,
            &mut anchors,
        )?;
    }
    Ok(())
}
fn validate_exact_numbers(value: &Value) -> Result<(), SchemaError> {
    match value {
        Value::Number(number) => crate::resolved::checked_number(number.as_str())
            .map(|_| ())
            .map_err(limit),
        Value::Array(values) => {
            for value in values {
                validate_exact_numbers(value)?
            }
            Ok(())
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_exact_numbers(value)?
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
#[derive(Clone, Copy)]
struct SchemaStats {
    work: usize,
    defaults: bool,
    uniqueness: bool,
    height: usize,
}

/// Preflight the same prepared resources the engine uses. Locations, reference
/// scope, default reachability, and expanded work must agree before compiling
/// any validators; a repeated acyclic reference graph can otherwise expand
/// exponentially despite having few physical schema nodes.
fn prepare_schema_graph(
    resources: &HashMap<String, Value>,
    registry: &Registry<'_>,
) -> Result<HashMap<SchemaLocation, SchemaStats>, SchemaError> {
    fn collect(value: &Value, resource: &str, pointer: &str, locations: &mut Vec<SchemaLocation>) {
        locations.push(SchemaLocation {
            resource: resource.into(),
            pointer: pointer.into(),
        });
        for child in profile::children(value) {
            collect(child.value, resource, &child.pointer(pointer), locations);
        }
    }
    struct Prepared<'a> {
        value: &'a Value,
        base: String,
        edges: Vec<(usize, bool, bool)>,
        location: SchemaLocation,
        own_work: usize,
    }
    let mut locations = Vec::new();
    let (mut total_nodes, mut total_bytes) = (0usize, 0usize);
    for (uri, schema) in resources {
        let (nodes, bytes) = check_value_limits(schema)?;
        total_nodes = total_nodes.saturating_add(nodes);
        total_bytes = total_bytes.saturating_add(bytes);
        collect(schema, uri, "", &mut locations);
    }
    if total_nodes > MAX_SCHEMA_NODES || total_bytes > MAX_SOURCE_BYTES {
        return Err(limit("composed schemas exceed aggregate node/byte bounds"));
    }
    let mut prepared = Vec::new();
    let mut addresses = HashMap::new();
    let mut copied_bytes = 0usize;
    for location in locations {
        let uri = jsonschema::uri::from_str(&location.resource)
            .map_err(|error| invalid(format!("invalid schema URI: {error}")))?;
        let resolved = registry
            .resolver(uri)
            .lookup(&canonical_reference(&location.resource, &location.pointer))
            .map_err(|error| unknown(format!("cannot locate schema view: {error}")))?;
        let value = resolved.contents();
        if !value.is_boolean() && !value.is_object() {
            return Err(
                invalid("a schema reference must identify an object or boolean schema")
                    .at(location),
            );
        }
        let (nodes, bytes) = check_value_limits(value)?;
        copied_bytes = copied_bytes.saturating_add(bytes);
        if copied_bytes > MAX_SOURCE_BYTES {
            return Err(limit("compiled schema views exceed aggregate byte bound"));
        }
        addresses.insert(value as *const Value as usize, prepared.len());
        prepared.push(Prepared {
            value,
            base: resolved.resolver().base_uri().as_str().to_owned(),
            location,
            edges: Vec::new(),
            own_work: nodes.saturating_add(bytes.div_ceil(64)),
        });
    }
    for node in &mut prepared {
        let mut edges = Vec::new();
        if let Some(reference) = node.value.get("$ref").and_then(Value::as_str) {
            let uri = jsonschema::uri::from_str(&node.base)
                .map_err(|error| invalid(format!("invalid reference base: {error}")))?;
            let resolved = registry.resolver(uri).lookup(reference).map_err(|error| {
                unknown(format!(
                    "unresolved schema reference {reference:?}: {error}"
                ))
            })?;
            let target = addresses
                .get(&(resolved.contents() as *const Value as usize))
                .copied()
                .ok_or_else(|| unknown("references into non-schema data are unsupported"))?;
            edges.push((target, false, true));
        }
        for child in profile::children(node.value) {
            let target = addresses
                .get(&(child.value as *const Value as usize))
                .copied()
                .ok_or_else(|| invalid("schema child is absent from the prepared registry"))?;
            edges.push((target, child.unsupported_defaults, child.applies));
        }
        node.edges = edges;
    }
    fn evaluate(
        id: usize,
        nodes: &[Prepared<'_>],
        active: &mut HashSet<usize>,
        memo: &mut [Option<SchemaStats>],
        depth: usize,
    ) -> Result<SchemaStats, SchemaError> {
        if active.contains(&id) {
            return Err(SchemaError::new(
                SchemaErrorKind::ReferenceCycle,
                "schema reference cycle",
            )
            .at(nodes[id].location.clone()));
        }
        if depth > MAX_SCHEMA_DEPTH {
            return Err(limit("expanded schema graph exceeds depth bound"));
        }
        if let Some(info) = memo[id] {
            return Ok(info);
        }
        active.insert(id);
        let mut info = SchemaStats {
            work: nodes[id].own_work,
            defaults: nodes[id].value.get("default").is_some(),
            uniqueness: nodes[id].value.get("uniqueItems") == Some(&Value::Bool(true)),
            height: 1,
        };
        for &(target, _restricted, applicable) in &nodes[id].edges {
            let child = evaluate(target, nodes, active, memo, depth + 1)?;
            if applicable {
                info.defaults |= child.defaults;
                info.uniqueness |= child.uniqueness;
            }
            info.work = info.work.saturating_add(child.work);
            info.height = info.height.max(child.height.saturating_add(1));
            if info.height > MAX_SCHEMA_DEPTH {
                return Err(limit("expanded schema graph exceeds depth bound"));
            }
            if info.work > MAX_COMPILE_NODES {
                return Err(limit("expanded schema evaluation work exceeds bound"));
            }
        }
        active.remove(&id);
        memo[id] = Some(info);
        Ok(info)
    }
    let mut memo = vec![None; prepared.len()];
    let mut active = HashSet::new();
    let mut total_work = 0usize;
    for id in 0..prepared.len() {
        total_work =
            total_work.saturating_add(evaluate(id, &prepared, &mut active, &mut memo, 0)?.work);
        if total_work > MAX_COMPILE_NODES {
            return Err(limit("aggregate expanded compilation work exceeds bound"));
        }
    }
    fn check_default_reachability(
        id: usize,
        nodes: &[Prepared<'_>],
        memo: &[Option<SchemaStats>],
        visited: &mut HashSet<usize>,
    ) -> Result<(), SchemaError> {
        if !visited.insert(id) {
            return Ok(());
        }
        for &(target, restricted, applicable) in &nodes[id].edges {
            if !applicable {
                continue;
            }
            if restricted
                && memo[target]
                    .expect("graph statistics were computed")
                    .defaults
            {
                return Err(SchemaError::new(
                    SchemaErrorKind::UnsupportedDefaults,
                    "defaults reachable through an unsupported applicator are unsupported",
                )
                .at(nodes[target].location.clone()));
            }
            check_default_reachability(target, nodes, memo, visited)?;
        }
        Ok(())
    }
    let mut reachable = HashSet::new();
    for (id, _) in prepared
        .iter()
        .enumerate()
        .filter(|(_, node)| node.location.pointer.is_empty())
    {
        check_default_reachability(id, &prepared, &memo, &mut reachable)?;
    }
    Ok(prepared
        .into_iter()
        .enumerate()
        .map(|(id, node)| (node.location, memo[id].expect("every view was evaluated")))
        .collect())
}
fn validate_default_bindings(
    schema: &Value,
    registered: &HashSet<&str>,
) -> Result<(), SchemaError> {
    if let Some(annotations) = schema.get("x-daml-default-tags").and_then(Value::as_array) {
        for annotation in annotations {
            if let Some(binding) = annotation.get("binding").and_then(Value::as_str)
                && !registered.contains(binding)
            {
                return Err(invalid(format!(
                    "default tag binding {binding:?} does not name a registered type"
                )));
            }
        }
    }
    for child in profile::children(schema) {
        validate_default_bindings(child.value, registered)?;
    }
    Ok(())
}
fn validate_default_tags(schema: &Map<String, Value>) -> Result<(), SchemaError> {
    let Some(annotations) = schema.get("x-daml-default-tags") else {
        return Ok(());
    };
    let default = schema
        .get("default")
        .ok_or_else(|| invalid("x-daml-default-tags requires a default on the same schema"))?;
    let annotations = annotations
        .as_array()
        .filter(|entries| !entries.is_empty())
        .ok_or_else(|| invalid("x-daml-default-tags must be a nonempty array"))?;
    let mut targets = HashSet::new();
    for annotation in annotations {
        let annotation = object(annotation, "default tag annotation")?;
        reject_unknown(
            annotation,
            &["path", "tag", "binding"],
            "default tag annotation",
        )?;
        let path = annotation
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("default tag annotation requires a string path"))?;
        let tag = annotation
            .get("tag")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("default tag annotation requires a string tag"))?;
        if let Some(binding) = annotation.get("binding") {
            let binding = binding
                .as_str()
                .ok_or_else(|| invalid("default tag binding must be a string"))?;
            valid_name(binding)?;
        }
        if !crate::names::valid_tag(tag) {
            return Err(invalid("default tag name is not a valid DAML tag spelling"));
        }
        if !path.is_empty() && !path.starts_with('/') {
            return Err(invalid("default tag paths must be RFC 6901 JSON Pointers"));
        }
        let mut bytes = path.bytes();
        while let Some(byte) = bytes.next() {
            if byte == b'~' && !matches!(bytes.next(), Some(b'0' | b'1')) {
                return Err(invalid(
                    "default tag paths contain an invalid JSON Pointer escape",
                ));
            }
        }
        let target = default.pointer(path).ok_or_else(|| {
            invalid(format!(
                "default tag path {path:?} does not identify a value"
            ))
        })?;
        if !targets.insert(target as *const Value as usize) {
            return Err(invalid(format!("duplicate default tag target {path:?}")));
        }
    }
    Ok(())
}

fn check_schema_shape(v: &Value) -> Result<(), SchemaError> {
    if !v.is_object() && !v.is_boolean() {
        return Err(invalid("schema must be an object or boolean"));
    }
    let mut n = 0;
    walk_schema(v, 0, &mut n)
}
fn walk_schema(v: &Value, depth: usize, n: &mut usize) -> Result<(), SchemaError> {
    *n += 1;
    if *n > MAX_SCHEMA_NODES || depth > MAX_SCHEMA_DEPTH {
        return Err(limit("schema exceeds node/depth bound"));
    }
    if let Some(m) = v.as_object() {
        if m.contains_key("x-daml-tag") || m.contains_key("x-daml-slot") {
            return Err(invalid(
                "x-daml-tag and x-daml-slot are unsupported schema keywords",
            ));
        }
        validate_default_tags(m)?;
        if let Some(optional) = m.get("x-daml-native-optional")
            && optional != &Value::Bool(true)
        {
            return Err(invalid("x-daml-native-optional must be true"));
        }
        if let Some(dialect) = m.get("$schema")
            && !matches!(
                dialect.as_str(),
                Some("https://json-schema.org/draft/2020-12/schema")
                    | Some("https://json-schema.org/draft/2020-12/schema#")
            )
        {
            return Err(invalid("$schema must select JSON Schema 2020-12"));
        }
        if m.contains_key("dependencies") || m.contains_key("additionalItems") {
            return Err(invalid(
                "legacy dependencies and additionalItems are unsupported",
            ));
        }
        if m.get("items").is_some_and(Value::is_array) {
            return Err(invalid("tuple-form items is unsupported"));
        }
        for k in ["$dynamicRef", "$recursiveRef"] {
            if m.contains_key(k) {
                return Err(invalid(format!("{k} is unsupported")));
            }
        }
        for child in profile::children(v) {
            walk_schema(child.value, depth + 1, n)?;
        }
    }
    Ok(())
}
fn validation_error_location(
    error: &ValidationError<'_>,
    fallback: &SchemaLocation,
) -> SchemaLocation {
    if let Some(uri) = error.absolute_keyword_location() {
        let (resource, fragment) = uri.as_str().split_once('#').unwrap_or((uri.as_str(), ""));
        return SchemaLocation {
            resource: resource.to_owned(),
            pointer: percent_decode(fragment).expect("keyword fragments encode UTF-8 schema paths"),
        };
    }
    SchemaLocation {
        resource: fallback.resource.clone(),
        pointer: error.schema_path().to_string(),
    }
}

fn native_regex_error_location(error: &ValidationError<'_>) -> Option<SchemaLocation> {
    let ValidationErrorKind::Custom { keyword, .. } = error.kind() else {
        return None;
    };
    if keyword != NATIVE_REGEX_KEYWORD {
        return None;
    }
    let instance = error.instance();
    let origin = instance.get("origin")?.as_u64()?;
    Some(SchemaLocation {
        resource: native_regex_origin(origin),
        pointer: String::new(),
    })
}

fn compile_error(error: ValidationError<'static>, fallback: SchemaLocation) -> SchemaError {
    let location = native_regex_error_location(&error)
        .unwrap_or_else(|| validation_error_location(&error, &fallback));
    SchemaError::new(
        SchemaErrorKind::InvalidSchema,
        format!("schema compilation failed: {error}"),
    )
    .at(location)
    .caused_by(error)
}
fn daml_value(d: &crate::Document, n: &crate::Node) -> Result<Value, SchemaError> {
    use crate::Value as V;
    if n.tag.is_some() {
        return Err(invalid(
            "tagged DAML schema documents are not JSON-compatible",
        ));
    }
    Ok(match &n.value {
        V::Null => Value::Null,
        V::Bool(x) => Value::Bool(*x),
        V::Number(span) => {
            let raw = d
                .slice(*span)
                .ok_or_else(|| invalid("invalid number span"))?;
            Value::Number(crate::resolved::checked_number(raw).map_err(limit)?)
        }
        V::String(_) => Value::String(
            d.string(n)
                .ok_or_else(|| invalid("invalid string"))?
                .to_owned(),
        ),
        V::Sequence(xs) => Value::Array(
            xs.iter()
                .map(|x| daml_value(d, x))
                .collect::<Result<_, _>>()?,
        ),
        V::Mapping(xs) => {
            let mut m = Map::new();
            for e in xs {
                let key = d.slice(e.key).ok_or_else(|| invalid("invalid key"))?;
                m.insert(key.to_owned(), daml_value(d, &e.value)?);
            }
            Value::Object(m)
        }
    })
}
