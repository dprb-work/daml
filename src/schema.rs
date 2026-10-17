use crate::{
    json_backend::{DamlJson, NodeRef},
    resolved::SchemaLocation,
    schema_error::{SchemaError, SchemaErrorKind},
    tag_keywords::{SlotKeyword, TagKeyword, TagRules},
};
use jsonschema::{Draft, PatternOptions, Registry, Resource, Validator};
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_SCHEMA_NODES: usize = 10_000;
const MAX_SCHEMA_DEPTH: usize = 128;
const MAX_COMPILE_NODES: usize = 100_000;

#[derive(Clone)]
struct Binding {
    value: Arc<Value>,
    id: String,
    legacy_defaults: bool,
    implements: Vec<String>,
}

/// A parsed schema module. Compose modules explicitly to create a registry.
#[derive(Clone, Default)]
pub struct SchemaModule {
    types: HashMap<String, Binding>,
    contracts: HashMap<String, Binding>,
    resources: HashMap<String, Value>,
}

impl SchemaModule {
    /// Loads a JSON-compatible module document.
    pub fn from_json(source: &str) -> Result<Self, SchemaError> {
        check_source(source)?;
        let value = crate::strict_json::parse(source).map_err(|error| {
            SchemaError::new(
                SchemaErrorKind::Parse,
                format!("invalid schema JSON: {error}"),
            )
            .caused_by(error)
        })?;
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
    /// # Errors
    ///
    /// Rejects malformed declarations and values exceeding schema bounds.
    pub fn from_value(value: Value) -> Result<Self, SchemaError> {
        check_value_limits(&value)?;
        let root = object(&value, "module")?;
        reject_unknown(root, &["types", "contracts", "resources"], "module")?;
        let mut module = Self::default();
        if let Some(types) = root.get("types") {
            for (name, item) in object(types, "types")? {
                valid_name(name)?;
                let item = object(item, "type binding")?;
                reject_unknown(item, &["schema", "implements"], "type binding")?;
                let schema = item
                    .get("schema")
                    .ok_or_else(|| invalid("type binding requires schema"))?;
                check_schema_shape(schema)?;
                let id = schema_id(schema).unwrap_or_else(|| generated_id("type", name));
                let mut value = schema.clone();
                assign_id(&mut value, &id)?;
                let implements = string_list(item.get("implements"), "implements")?;
                module.types.insert(
                    name.clone(),
                    Binding {
                        value: Arc::new(value),
                        id,
                        legacy_defaults: false,
                        implements,
                    },
                );
            }
        }
        if let Some(contracts) = root.get("contracts") {
            for (name, schema) in object(contracts, "contracts")? {
                valid_name(name)?;
                check_schema_shape(schema)?;
                let id = schema_id(schema).unwrap_or_else(|| generated_id("contract", name));
                let mut value = schema.clone();
                assign_id(&mut value, &id)?;
                module.contracts.insert(
                    name.clone(),
                    Binding {
                        value: Arc::new(value),
                        id,
                        legacy_defaults: false,
                        implements: Vec::new(),
                    },
                );
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

    /// Imports one Olaf JSON Schema as a DAML type, lowering discriminator
    /// properties only in this explicit adapter.
    pub fn from_olaf_json(tag: &str, source: &str) -> Result<Self, SchemaError> {
        check_source(source)?;
        valid_name(tag)?;
        let mut value = crate::strict_json::parse(source).map_err(|error| {
            SchemaError::new(
                SchemaErrorKind::Parse,
                format!("invalid Olaf schema JSON: {error}"),
            )
            .caused_by(error)
        })?;
        check_schema_shape(&value)?;
        check_value_limits(&value)?;
        let root_aliases = olaf_root_identities(&value)?;
        if !root_aliases.is_empty() && !root_aliases.iter().any(|name| name == tag) {
            return Err(invalid(format!(
                "Olaf import tag {tag:?} is not a declared root identity"
            )));
        }
        let aliases = if root_aliases.is_empty() {
            HashSet::from([tag.to_owned()])
        } else {
            root_aliases.into_iter().collect()
        };
        lower_olaf_discriminators(&mut value)?;
        let id = schema_id(&value).unwrap_or_else(|| generated_id("type", tag));
        assign_id(&mut value, &id)?;
        let mut module = Self::default();
        let value = Arc::new(value);
        for alias in aliases {
            module.types.insert(
                alias,
                Binding {
                    value: Arc::clone(&value),
                    id: id.clone(),
                    legacy_defaults: true,
                    implements: Vec::new(),
                },
            );
        }
        Ok(module)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SchemaView<'a> {
    pub id: usize,
    pub value: &'a Value,
    pub location: &'a SchemaLocation,
    pub default_origin: Option<&'a SchemaLocation>,
    pub legacy_defaults: bool,
}
struct SchemaEntry {
    value: Value,
    location: SchemaLocation,
    default_origin: Option<SchemaLocation>,
    legacy_defaults: bool,
    validator: Validator<DamlJson>,
    work: usize,
    has_defaults: bool,
}

/// Immutable, explicitly composed schema registry and its compiled validators.
pub struct Schemas {
    entries: Vec<SchemaEntry>,
    by_type: HashMap<String, usize>,
    by_address: HashMap<usize, usize>,
    references: HashMap<usize, usize>,
}

impl Schemas {
    /// Composes modules without last-writer-wins behavior.
    pub fn compose(modules: impl IntoIterator<Item = SchemaModule>) -> Result<Self, SchemaError> {
        let mut types = HashMap::<String, Binding>::new();
        let mut contracts = HashMap::<String, Binding>::new();
        let mut resources = HashMap::<String, Value>::new();
        let (mut input_nodes, mut input_bytes) = (0usize, 0usize);
        let mut seen_values = HashSet::new();
        for module in modules {
            for (name, binding) in module.types.iter().chain(module.contracts.iter()) {
                input_nodes = input_nodes.saturating_add(1);
                input_bytes = input_bytes
                    .saturating_add(name.len())
                    .saturating_add(binding.id.len());
                for name in &binding.implements {
                    input_nodes = input_nodes.saturating_add(1);
                    input_bytes = input_bytes.saturating_add(name.len());
                }
                if seen_values.insert(Arc::as_ptr(&binding.value) as usize) {
                    let (nodes, bytes) = check_value_limits(&binding.value)?;
                    input_nodes = input_nodes.saturating_add(nodes);
                    input_bytes = input_bytes.saturating_add(bytes);
                }
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
                    return Err(duplicate(format!("duplicate type or alias {name:?}")));
                }
            }
            for (name, value) in module.contracts {
                if contracts.insert(name.clone(), value).is_some() {
                    return Err(duplicate(format!("duplicate contract {name:?}")));
                }
            }
            for (uri, value) in module.resources {
                if resources.insert(uri.clone(), value).is_some() {
                    return Err(duplicate(format!("duplicate resource URI {uri:?}")));
                }
            }
        }
        normalize_olaf_resources(&mut types, &mut resources)?;
        let mut identifiers = HashMap::<String, Arc<Value>>::new();
        for binding in types.values().chain(contracts.values()) {
            if let Some(previous) =
                identifiers.insert(binding.id.clone(), Arc::clone(&binding.value))
                && !Arc::ptr_eq(&previous, &binding.value)
            {
                return Err(duplicate(format!(
                    "duplicate schema identifier {:?}",
                    binding.id
                )));
            }
        }
        for (uri, schema) in &resources {
            if identifiers.contains_key(uri) {
                return Err(duplicate(format!("duplicate schema identifier {uri:?}")));
            }
            check_schema_shape(schema)?;
        }
        let mut memberships = HashMap::new();
        for (tag, binding) in &types {
            let mut contracts_for_tag = HashSet::new();
            for name in &binding.implements {
                if !contracts.contains_key(name) {
                    return Err(unknown(format!("unknown implemented contract {name:?}")));
                }
                contracts_for_tag.insert(name.clone());
            }
            memberships.insert(tag.clone(), contracts_for_tag);
        }
        let rules = Arc::new(TagRules { memberships });
        drop(identifiers);
        let contract_ids = contracts
            .iter()
            .map(|(name, binding)| (name.clone(), binding.id.clone()))
            .collect::<HashMap<_, _>>();
        let mut added_reference_bytes = 0;
        for binding in contracts.values_mut() {
            expand_contracts(
                Arc::make_mut(&mut binding.value),
                &contract_ids,
                &mut added_reference_bytes,
            )?;
        }
        for schema in resources.values_mut() {
            expand_contracts(schema, &contract_ids, &mut added_reference_bytes)?;
        }
        let mut expanded = HashMap::<String, Arc<Value>>::new();
        for binding in types.values_mut() {
            if let Some(value) = expanded.get(&binding.id) {
                binding.value = Arc::clone(value);
                continue;
            }
            let root_value = Arc::make_mut(&mut binding.value);
            expand_contracts(root_value, &contract_ids, &mut added_reference_bytes)?;
            for name in &binding.implements {
                let uri = contract_ids
                    .get(name)
                    .ok_or_else(|| unknown(format!("unknown implemented contract {name:?}")))?;
                added_reference_bytes = added_reference_bytes.saturating_add(uri.len());
                if added_reference_bytes > MAX_SOURCE_BYTES {
                    return Err(limit(
                        "expanded contract references exceed schema byte bound",
                    ));
                }
                let root = root_value
                    .as_object_mut()
                    .ok_or_else(|| invalid("type schemas with implements must be objects"))?;
                root.entry("allOf")
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .ok_or_else(|| invalid("allOf must be an array"))?
                    .push(serde_json::json!({"$ref": uri}));
            }
            expanded.insert(binding.id.clone(), Arc::clone(&binding.value));
        }
        let mut registry_contents = resources.clone();
        for binding in types.values().chain(contracts.values()) {
            registry_contents
                .entry(binding.id.clone())
                .or_insert_with(|| binding.value.as_ref().clone());
        }
        validate_nested_identifiers(&registry_contents)?;
        for schema in types
            .values()
            .map(|b| b.value.as_ref())
            .chain(contracts.values().map(|b| b.value.as_ref()))
            .chain(resources.values())
        {
            validate_exact_numbers(schema)?;
            validate_default_placements(schema, false)?;
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
            .with_pattern_options(
                PatternOptions::regex()
                    .size_limit(1 << 20)
                    .dfa_size_limit(1 << 20),
            )
            .with_keyword("x-daml-tag", |_, value, _| {
                TagKeyword::new(value)
                    .map(|k| Box::new(k) as _)
                    .map_err(jsonschema::ValidationError::custom)
            })
            .with_keyword("x-daml-slot", {
                let rules = Arc::clone(&rules);
                move |_, value, _| {
                    SlotKeyword::new(value, Arc::clone(&rules))
                        .map(|k| Box::new(k) as _)
                        .map_err(jsonschema::ValidationError::custom)
                }
            });
        let mut entries = Vec::new();
        let mut by_type = HashMap::new();
        let mut total = 0usize;
        let mut compiled_ids = HashMap::<String, usize>::new();
        for (name, binding) in types {
            let index = if let Some(index) = compiled_ids.get(&binding.id) {
                *index
            } else {
                let index = entries.len();
                append_schema_entries(
                    binding.value.as_ref().clone(),
                    &binding.id,
                    binding.legacy_defaults,
                    &options,
                    &mut entries,
                    &mut total,
                )?;
                compiled_ids.insert(binding.id.clone(), index);
                index
            };
            by_type.insert(name, index);
        }
        for (_, binding) in contracts {
            append_schema_entries(
                binding.value.as_ref().clone(),
                &binding.id,
                false,
                &options,
                &mut entries,
                &mut total,
            )?;
        }
        for (uri, value) in resources {
            append_schema_entries(value, &uri, false, &options, &mut entries, &mut total)?;
        }
        let lookup = entries
            .iter()
            .enumerate()
            .map(|(id, e)| {
                (
                    (e.location.resource.clone(), e.location.pointer.clone()),
                    id,
                )
            })
            .collect::<HashMap<_, _>>();
        for entry in &mut entries {
            let info = stats
                .get(&entry.location)
                .expect("every compiled view was preflighted");
            entry.work = info.work;
            entry.has_defaults = info.defaults;
        }
        let mut by_address = HashMap::new();
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
            by_address.insert(target.contents() as *const Value as usize, id);
        }
        for (id, entry) in entries.iter().enumerate() {
            index_nested_addresses(
                &entry.value,
                &entry.location.resource,
                &entry.location.pointer,
                id,
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
            let target_id = by_address
                .get(&(target.contents() as *const Value as usize))
                .copied()
                .expect("preflight admitted only references to indexed schema nodes");
            references.insert(id, target_id);
        }
        Ok(Self {
            entries,
            by_type,
            by_address,
            references,
        })
    }

    pub(crate) fn type_schema(&self, tag: &str) -> Option<SchemaView<'_>> {
        self.by_type.get(tag).copied().map(|id| self.view(id))
    }
    pub(crate) fn view(&self, id: usize) -> SchemaView<'_> {
        let e = &self.entries[id];
        SchemaView {
            id,
            value: &e.value,
            location: &e.location,
            default_origin: e.default_origin.as_ref(),
            legacy_defaults: e.legacy_defaults,
        }
    }
    pub(crate) fn locate(&self, value: &Value) -> Option<SchemaView<'_>> {
        self.by_address
            .get(&(value as *const Value as usize))
            .copied()
            .map(|id| self.view(id))
    }
    pub(crate) fn referenced(&self, view: SchemaView<'_>) -> Option<SchemaView<'_>> {
        self.references
            .get(&view.id)
            .copied()
            .map(|target| self.view(target))
    }
    pub(crate) fn check(
        &self,
        node: NodeRef<'_>,
        schema: SchemaView<'_>,
    ) -> Result<(), SchemaFailure> {
        let Some(entry) = self.entries.get(schema.id) else {
            return Err(SchemaFailure {
                path: String::new(),
                schema: schema.location.clone(),
                message: "schema view does not belong to this registry".into(),
                tag: false,
            });
        };
        entry.validator.validate(node).map_err(|error| {
            let keyword = error.schema_path().to_string();
            let tag = keyword.ends_with("/x-daml-tag") || keyword.ends_with("/x-daml-slot");
            let schema_location = if let Some(uri) = error.absolute_keyword_location() {
                let (resource, fragment) =
                    uri.as_str().split_once('#').unwrap_or((uri.as_str(), ""));
                SchemaLocation {
                    resource: resource.to_owned(),
                    pointer: percent_decode(fragment)
                        .expect("keyword fragments encode UTF-8 schema paths"),
                }
            } else {
                SchemaLocation {
                    resource: schema.location.resource.clone(),
                    pointer: keyword,
                }
            };
            SchemaFailure {
                path: error.instance_path().to_string(),
                schema: schema_location,
                message: error.masked().to_string(),
                tag,
            }
        })
    }
    /// Conservative schema-work estimate for charging repeated runtime checks.
    pub(crate) fn check_work(&self, view: SchemaView<'_>) -> usize {
        self.entries
            .get(view.id)
            .map_or(usize::MAX, |entry| entry.work)
    }
    pub(crate) fn evaluation_cost(&self, view: SchemaView<'_>, nodes: usize) -> usize {
        self.check_work(view).saturating_mul(nodes)
    }
    pub(crate) fn has_defaults(&self, view: SchemaView<'_>) -> bool {
        self.entries[view.id].has_defaults
    }
}

pub(crate) struct SchemaFailure {
    pub path: String,
    pub schema: SchemaLocation,
    pub message: String,
    pub tag: bool,
}
fn index_nested_addresses(
    value: &Value,
    resource: &str,
    pointer: &str,
    id: usize,
    lookup: &HashMap<(String, String), usize>,
    addresses: &mut HashMap<usize, usize>,
) {
    addresses.insert(value as *const Value as usize, id);
    if let Some(map) = value.as_object() {
        for (key, child) in map {
            if schema_map_key(key) {
                for (name, schema) in child.as_object().into_iter().flat_map(|m| m.iter()) {
                    let path = pointer_join(&pointer_join(pointer, key), name);
                    let child_id = lookup
                        .get(&(resource.to_owned(), path.clone()))
                        .copied()
                        .unwrap_or(id);
                    index_nested_addresses(schema, resource, &path, child_id, lookup, addresses);
                }
            } else if schema_array_key(key) {
                for (index, schema) in child.as_array().into_iter().flatten().enumerate() {
                    let path = pointer_join(&pointer_join(pointer, key), &index.to_string());
                    let child_id = lookup
                        .get(&(resource.to_owned(), path.clone()))
                        .copied()
                        .unwrap_or(id);
                    index_nested_addresses(schema, resource, &path, child_id, lookup, addresses);
                }
            } else if schema_single_key(key) && (child.is_object() || child.is_boolean()) {
                let path = pointer_join(pointer, key);
                let child_id = lookup
                    .get(&(resource.to_owned(), path.clone()))
                    .copied()
                    .unwrap_or(id);
                index_nested_addresses(child, resource, &path, child_id, lookup, addresses);
            }
        }
    }
}

fn append_schema_entries(
    value: Value,
    resource: &str,
    legacy: bool,
    options: &jsonschema::ValidationOptions<'_, std::sync::Arc<dyn jsonschema::Retrieve>, DamlJson>,
    entries: &mut Vec<SchemaEntry>,
    total: &mut usize,
) -> Result<(), SchemaError> {
    fn visit(
        value: &Value,
        resource: &str,
        pointer: &str,
        legacy: bool,
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
            pointer: pointer_join(pointer, "default"),
        });
        entries.push(SchemaEntry {
            value: value.clone(),
            location,
            default_origin,
            legacy_defaults: legacy,
            validator,
            work: 0,
            has_defaults: false,
        });
        for (key, child) in value.as_object().into_iter().flat_map(|m| m.iter()) {
            if schema_map_key(key) {
                for (name, schema) in child.as_object().into_iter().flat_map(|m| m.iter()) {
                    visit(
                        schema,
                        resource,
                        &pointer_join(&pointer_join(pointer, key), name),
                        legacy,
                        options,
                        entries,
                        total,
                    )?;
                }
            } else if schema_array_key(key) {
                for (n, schema) in child.as_array().into_iter().flatten().enumerate() {
                    visit(
                        schema,
                        resource,
                        &pointer_join(&pointer_join(pointer, key), &n.to_string()),
                        legacy,
                        options,
                        entries,
                        total,
                    )?;
                }
            } else if schema_single_key(key) && (child.is_object() || child.is_boolean()) {
                visit(
                    child,
                    resource,
                    &pointer_join(pointer, key),
                    legacy,
                    options,
                    entries,
                    total,
                )?;
            }
        }
        Ok(())
    }
    let mut nodes = 0;
    walk_schema(&value, 0, &mut nodes, &mut |_, _| Ok(()))?;
    visit(&value, resource, "", legacy, options, entries, total)
}
fn schema_map_key(k: &str) -> bool {
    matches!(
        k,
        "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas"
    )
}
fn schema_array_key(k: &str) -> bool {
    matches!(k, "allOf" | "anyOf" | "oneOf" | "prefixItems")
}
fn schema_single_key(k: &str) -> bool {
    matches!(
        k,
        "items"
            | "additionalProperties"
            | "additionalItems"
            | "not"
            | "if"
            | "then"
            | "else"
            | "contains"
            | "propertyNames"
            | "unevaluatedProperties"
            | "unevaluatedItems"
            | "contentSchema"
    )
}
fn pointer_join(base: &str, token: &str) -> String {
    let token = token.replace('~', "~0").replace('/', "~1");
    format!("{base}/{token}")
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
    let _ = walk_schema(value, 0, &mut count, &mut |_, _| Ok(()));
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
fn normalize_olaf_resources(
    types: &mut HashMap<String, Binding>,
    resources: &mut HashMap<String, Value>,
) -> Result<(), SchemaError> {
    let aliases = resources
        .keys()
        .filter(|uri| uri.starts_with("@/"))
        .map(|uri| (uri.clone(), olaf_resource_uri(uri)))
        .collect::<HashMap<_, _>>();
    for binding in types.values_mut().filter(|binding| binding.legacy_defaults) {
        rewrite_olaf_references(Arc::make_mut(&mut binding.value), &aliases);
    }
    let mut normalized = HashMap::with_capacity(resources.len());
    for (uri, mut schema) in std::mem::take(resources) {
        let target = aliases.get(&uri).cloned().unwrap_or(uri.clone());
        if let Some(map) = schema.as_object_mut()
            && map.get("$id").and_then(Value::as_str) == Some(uri.as_str())
        {
            map.insert("$id".into(), Value::String(target.clone()));
        }
        rewrite_olaf_references(&mut schema, &aliases);
        if normalized.insert(target.clone(), schema).is_some() {
            return Err(duplicate(format!(
                "duplicate normalized Olaf resource URI {target:?}"
            )));
        }
    }
    *resources = normalized;
    Ok(())
}
fn olaf_resource_uri(uri: &str) -> String {
    let hex = uri
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("urn:daml:olaf:{hex}")
}
fn rewrite_olaf_references(schema: &mut Value, aliases: &HashMap<String, String>) {
    let Some(map) = schema.as_object_mut() else {
        return;
    };
    if let Some(reference) = map.get("$ref").and_then(Value::as_str).map(str::to_owned) {
        let (resource, fragment) = reference.split_once('#').unwrap_or((&reference, ""));
        if let Some(normalized) = aliases.get(resource) {
            map.insert(
                "$ref".into(),
                Value::String(format!("{normalized}#{fragment}")),
            );
        }
    }
    for (key, child) in map.iter_mut() {
        if schema_map_key(key) {
            if let Some(children) = child.as_object_mut() {
                for value in children.values_mut() {
                    rewrite_olaf_references(value, aliases)
                }
            }
        } else if schema_array_key(key) {
            if let Some(children) = child.as_array_mut() {
                for value in children {
                    rewrite_olaf_references(value, aliases)
                }
            }
        } else if schema_single_key(key) {
            rewrite_olaf_references(child, aliases)
        }
    }
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
fn string_list(v: Option<&Value>, label: &str) -> Result<Vec<String>, SchemaError> {
    let Some(v) = v else { return Ok(Vec::new()) };
    let a = v
        .as_array()
        .ok_or_else(|| invalid(format!("{label} must be an array")))?;
    let mut set = HashSet::new();
    a.iter()
        .map(|v| {
            let s = v
                .as_str()
                .ok_or_else(|| invalid(format!("{label} entries must be strings")))?;
            valid_name(s)?;
            if !set.insert(s) {
                return Err(duplicate(format!("duplicate {label} entry {s:?}")));
            }
            Ok(s.to_owned())
        })
        .collect()
}
fn generated_id(kind: &str, name: &str) -> String {
    format!(
        "urn:daml:{kind}:{}",
        name.as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
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
        for (key, child) in map {
            if schema_map_key(key) {
                for (name, child) in child.as_object().into_iter().flat_map(|map| map.iter()) {
                    visit(
                        child,
                        &scope,
                        &SchemaLocation {
                            resource: location.resource.clone(),
                            pointer: pointer_join(&pointer_join(&location.pointer, key), name),
                        },
                        ids,
                        anchors,
                    )?;
                }
            } else if schema_array_key(key) {
                for (index, child) in child.as_array().into_iter().flatten().enumerate() {
                    visit(
                        child,
                        &scope,
                        &SchemaLocation {
                            resource: location.resource.clone(),
                            pointer: pointer_join(
                                &pointer_join(&location.pointer, key),
                                &index.to_string(),
                            ),
                        },
                        ids,
                        anchors,
                    )?;
                }
            } else if schema_single_key(key) && (child.is_object() || child.is_boolean()) {
                visit(
                    child,
                    &scope,
                    &SchemaLocation {
                        resource: location.resource.clone(),
                        pointer: pointer_join(&location.pointer, key),
                    },
                    ids,
                    anchors,
                )?;
            }
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
        for (key, child) in value.as_object().into_iter().flat_map(|map| map.iter()) {
            if schema_map_key(key) {
                for (name, child) in child.as_object().into_iter().flat_map(|map| map.iter()) {
                    collect(
                        child,
                        resource,
                        &pointer_join(&pointer_join(pointer, key), name),
                        locations,
                    );
                }
            } else if schema_array_key(key) {
                for (index, child) in child.as_array().into_iter().flatten().enumerate() {
                    collect(
                        child,
                        resource,
                        &pointer_join(&pointer_join(pointer, key), &index.to_string()),
                        locations,
                    );
                }
            } else if schema_single_key(key) && (child.is_object() || child.is_boolean()) {
                collect(child, resource, &pointer_join(pointer, key), locations);
            }
        }
    }
    struct Prepared<'a> {
        value: &'a Value,
        base: String,
        location: SchemaLocation,
        edges: Vec<(usize, bool)>,
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
            edges.push((target, false));
        }
        let pattern_properties = node.value.get("patternProperties").is_some();
        for (key, child) in node
            .value
            .as_object()
            .into_iter()
            .flat_map(|map| map.iter())
        {
            let restricted = matches!(
                key.as_str(),
                "patternProperties"
                    | "dependentSchemas"
                    | "not"
                    | "contains"
                    | "propertyNames"
                    | "if"
                    | "then"
                    | "else"
                    | "unevaluatedProperties"
                    | "unevaluatedItems"
                    | "additionalItems"
                    | "contentSchema"
            ) || (key == "additionalProperties" && pattern_properties);
            let mut add = |value: &Value| -> Result<(), SchemaError> {
                let target = addresses
                    .get(&(value as *const Value as usize))
                    .copied()
                    .ok_or_else(|| invalid("schema child is absent from the prepared registry"))?;
                edges.push((target, restricted));
                Ok(())
            };
            if schema_map_key(key) {
                for child in child.as_object().into_iter().flat_map(|map| map.values()) {
                    add(child)?;
                }
            } else if schema_array_key(key) {
                for child in child.as_array().into_iter().flatten() {
                    add(child)?;
                }
            } else if schema_single_key(key) && (child.is_object() || child.is_boolean()) {
                add(child)?;
            }
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
            height: 1,
        };
        for &(target, restricted) in &nodes[id].edges {
            let child = evaluate(target, nodes, active, memo, depth + 1)?;
            if restricted && child.defaults {
                return Err(SchemaError::new(
                    SchemaErrorKind::UnsupportedDefaults,
                    "defaults reachable through an unsupported applicator are unsupported",
                )
                .at(nodes[target].location.clone()));
            }
            info.defaults |= child.defaults;
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
    Ok(prepared
        .into_iter()
        .enumerate()
        .map(|(id, node)| (node.location, memo[id].expect("every view was evaluated")))
        .collect())
}
fn validate_default_placements(schema: &Value, unsupported: bool) -> Result<(), SchemaError> {
    if unsupported && schema.get("default").is_some() {
        return Err(SchemaError::new(
            SchemaErrorKind::UnsupportedDefaults,
            "defaults under this applicator are unsupported",
        ));
    }
    let Some(map) = schema.as_object() else {
        return Ok(());
    };
    for (key, value) in map {
        let nested_unsupported = unsupported
            || matches!(
                key.as_str(),
                "patternProperties"
                    | "dependentSchemas"
                    | "not"
                    | "contains"
                    | "propertyNames"
                    | "if"
                    | "then"
                    | "else"
                    | "unevaluatedProperties"
                    | "unevaluatedItems"
                    | "additionalItems"
                    | "contentSchema"
            )
            || (key == "additionalProperties" && map.contains_key("patternProperties"));
        if schema_map_key(key) {
            for child in value.as_object().into_iter().flat_map(|m| m.values()) {
                validate_default_placements(child, nested_unsupported)?
            }
        } else if schema_array_key(key) {
            for child in value.as_array().into_iter().flatten() {
                validate_default_placements(child, nested_unsupported)?
            }
        } else if schema_single_key(key) && (value.is_object() || value.is_boolean()) {
            validate_default_placements(value, nested_unsupported)?
        }
    }
    Ok(())
}
fn check_schema_shape(v: &Value) -> Result<(), SchemaError> {
    if !v.is_object() && !v.is_boolean() {
        return Err(invalid("schema must be an object or boolean"));
    }
    let mut n = 0;
    walk_schema(v, 0, &mut n, &mut |_, _| Ok(()))
}
fn walk_schema(
    v: &Value,
    depth: usize,
    n: &mut usize,
    visit: &mut impl FnMut(&Value, &str) -> Result<(), SchemaError>,
) -> Result<(), SchemaError> {
    *n += 1;
    if *n > MAX_SCHEMA_NODES || depth > MAX_SCHEMA_DEPTH {
        return Err(limit("schema exceeds node/depth bound"));
    }
    if let Some(m) = v.as_object() {
        for k in ["$dynamicRef", "$recursiveRef"] {
            if m.contains_key(k) {
                return Err(invalid(format!("{k} is unsupported")));
            }
        }
        for (k, c) in m {
            visit(c, k)?;
            if schema_map_key(k) {
                for x in c.as_object().into_iter().flat_map(|x| x.values()) {
                    walk_schema(x, depth + 1, n, visit)?
                }
            } else if schema_array_key(k) {
                for x in c.as_array().into_iter().flatten() {
                    walk_schema(x, depth + 1, n, visit)?
                }
            } else if schema_single_key(k) && (c.is_object() || c.is_boolean()) {
                walk_schema(c, depth + 1, n, visit)?
            }
        }
    }
    Ok(())
}
fn compile_error(
    e: impl std::error::Error + Send + Sync + 'static,
    l: SchemaLocation,
) -> SchemaError {
    SchemaError::new(
        SchemaErrorKind::InvalidSchema,
        format!("schema compilation failed: {e}"),
    )
    .at(l)
    .caused_by(e)
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
fn marker_values(marker: &Value, key: &str) -> Result<Vec<String>, SchemaError> {
    let values = if let Some(s) = marker.get("const").and_then(Value::as_str) {
        vec![s.to_owned()]
    } else if let Some(values) = marker.get("enum").and_then(Value::as_array) {
        if values.is_empty() || values.iter().any(|value| value.as_str().is_none()) {
            return Err(invalid(format!(
                "Olaf {key} enum must contain nonempty strings"
            )));
        }
        values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    } else {
        return Err(invalid(format!(
            "Olaf {key} marker must declare string const or enum"
        )));
    };
    if values.iter().any(String::is_empty) {
        return Err(invalid("Olaf discriminator values must be nonempty"));
    }
    let mut unique = HashSet::new();
    if values.iter().any(|value| !unique.insert(value)) {
        return Err(invalid(
            "Olaf discriminator marker contains duplicate values",
        ));
    }
    Ok(values)
}
fn olaf_root_identities(schema: &Value) -> Result<Vec<String>, SchemaError> {
    let mut current = schema;
    let mut seen = HashSet::new();
    while let Some(reference) = current.get("$ref").and_then(Value::as_str) {
        let Some(fragment) = reference.strip_prefix('#') else {
            return Ok(Vec::new());
        };
        if !seen.insert(reference) {
            return Err(SchemaError::new(
                SchemaErrorKind::ReferenceCycle,
                "cyclic Olaf root reference",
            ));
        }
        let pointer =
            percent_decode(fragment).ok_or_else(|| invalid("invalid Olaf root reference"))?;
        current = schema
            .pointer(&pointer)
            .ok_or_else(|| unknown("unresolved Olaf root reference"))?;
    }
    let Some(properties) = current.get("properties").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let markers = ["__type__", "__fn__"]
        .into_iter()
        .filter_map(|key| properties.get(key).map(|value| (key, value)))
        .collect::<Vec<_>>();
    if markers.len() > 1 {
        return Err(invalid(
            "Olaf root has ambiguous __type__ and __fn__ identities",
        ));
    }
    markers
        .into_iter()
        .try_fold(Vec::new(), |mut names, (key, marker)| {
            names.extend(marker_values(marker, key)?);
            Ok(names)
        })
}

fn lower_olaf_discriminators(value: &mut Value) -> Result<(), SchemaError> {
    let Some(map) = value.as_object_mut() else {
        return Ok(());
    };
    let mut allowed = Vec::new();
    let mut removed = Vec::new();
    if let Some(properties) = map.get_mut("properties").and_then(Value::as_object_mut) {
        for key in ["__type__", "__fn__"] {
            if let Some(marker) = properties.remove(key) {
                if !removed.is_empty() {
                    return Err(invalid("Olaf schema declares conflicting identity fields"));
                }
                allowed.extend(marker_values(&marker, key)?);
                removed.push(key);
            }
        }
    }
    if let Some(required) = map.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|name| !name.as_str().is_some_and(|name| removed.contains(&name)));
    }
    if !allowed.is_empty() {
        if map.contains_key("x-daml-tag") {
            return Err(invalid(
                "Olaf identity and x-daml-tag cannot both declare a discriminator",
            ));
        }
        map.insert(
            "x-daml-tag".into(),
            Value::Array(allowed.into_iter().map(Value::String).collect()),
        );
    }
    for (key, children) in map.iter_mut() {
        if schema_map_key(key) {
            if let Some(children) = children.as_object_mut() {
                for child in children.values_mut() {
                    lower_olaf_discriminators(child)?;
                }
            }
        } else if schema_array_key(key) {
            if let Some(children) = children.as_array_mut() {
                for child in children {
                    lower_olaf_discriminators(child)?;
                }
            }
        } else if schema_single_key(key) {
            lower_olaf_discriminators(children)?;
        }
    }
    Ok(())
}
fn expand_contracts(
    value: &mut Value,
    contracts: &HashMap<String, String>,
    added_bytes: &mut usize,
) -> Result<(), SchemaError> {
    let Some(map) = value.as_object_mut() else {
        return Ok(());
    };
    if let Some(slot) = map.get("x-daml-slot") {
        let name = slot
            .as_str()
            .ok_or_else(|| invalid("x-daml-slot must be a contract-name string"))?;
        let uri = contracts
            .get(name)
            .ok_or_else(|| unknown(format!("unknown contract slot {name:?}")))?;
        *added_bytes = added_bytes.saturating_add(uri.len());
        if *added_bytes > MAX_SOURCE_BYTES {
            return Err(limit(
                "expanded contract references exceed schema byte bound",
            ));
        }
        map.entry("allOf")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| invalid("allOf must be an array"))?
            .push(serde_json::json!({"$ref": uri}));
    }
    for (key, children) in map.iter_mut() {
        if schema_map_key(key) {
            if let Some(children) = children.as_object_mut() {
                for child in children.values_mut() {
                    expand_contracts(child, contracts, added_bytes)?;
                }
            }
        } else if schema_array_key(key) {
            if let Some(children) = children.as_array_mut() {
                for child in children {
                    expand_contracts(child, contracts, added_bytes)?;
                }
            }
        } else if schema_single_key(key) {
            expand_contracts(children, contracts, added_bytes)?;
        }
    }
    Ok(())
}
