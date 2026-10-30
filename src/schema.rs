use crate::{
    json_backend::{DamlJson, NodeRef},
    resolved::{ResolvedNode, ResolvedValue, SchemaLocation},
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
    pub(crate) fn binding(self) -> std::borrow::Cow<'a, str> {
        let location = self.location();
        if location.pointer.is_empty() {
            std::borrow::Cow::Borrowed(&location.resource)
        } else {
            std::borrow::Cow::Owned(canonical_reference(&location.resource, &location.pointer))
        }
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
    /// Reserve an upper bound before calling the upstream engine. This walks
    /// structure, never decides whether a constraint or branch is valid.
    pub(crate) fn evaluation_cost(self, node: &ResolvedNode<'_>, limit: usize) -> usize {
        fn measure(
            node: &ResolvedNode<'_>,
            uniqueness: bool,
            record: &mut impl FnMut(usize, usize, usize, usize),
        ) -> (usize, usize) {
            let (units, unique, immediate) = match &node.value {
                ResolvedValue::Mapping(fields) => fields.iter().fold(
                    (1usize, 0usize, 1usize),
                    |(units, unique, immediate), (key, child)| {
                        let (child_units, child_unique) = measure(child, uniqueness, record);
                        (
                            units
                                .saturating_add(key.len().div_ceil(64))
                                .saturating_add(child_units),
                            unique.saturating_add(child_unique),
                            immediate.saturating_add(key.len().div_ceil(64).max(1)),
                        )
                    },
                ),
                ResolvedValue::Sequence(items) => {
                    let (mut units, mut unique, mut largest) = (1usize, 0usize, 0usize);
                    for child in items {
                        let (child_units, child_unique) = measure(child, uniqueness, record);
                        units = units.saturating_add(child_units);
                        unique = unique.saturating_add(child_unique);
                        largest = largest.max(child_units);
                    }
                    let unique = if uniqueness {
                        let pairs = items.len().saturating_mul(items.len().saturating_sub(1)) / 2;
                        unique.saturating_add(pairs.saturating_mul(largest))
                    } else {
                        0
                    };
                    (units, unique, 1usize.saturating_add(items.len()))
                }
                ResolvedValue::String(value) => {
                    let units = 1usize.saturating_add(value.len().div_ceil(64));
                    (units, 0, units)
                }
                ResolvedValue::Number(value) => {
                    let units = 1usize.saturating_add(value.as_ref().as_str().len().div_ceil(64));
                    (units, 0, units)
                }
                _ => (1, 0, 1),
            };
            record(
                node as *const ResolvedNode<'_> as usize,
                units,
                unique,
                immediate,
            );
            (units, unique)
        }
        if let Some(work) = self.entry().work {
            let (units, unique) = measure(node, self.entry().uses_uniqueness, &mut |_, _, _, _| {});
            // Keep the admitted acyclic profile's existing cost and limits.
            return work.saturating_mul(units.saturating_add(if self.entry().uses_uniqueness {
                unique
            } else {
                0
            }));
        }
        let mut measured = HashMap::new();
        measure(
            node,
            self.entry().uses_uniqueness,
            &mut |address, units, unique, immediate| {
                measured.insert(address, (units, unique, immediate));
            },
        );
        // Errors covers validate/collect_errors; Validity covers is_valid.
        // An error-producing union probes and then collects, but its probes do
        // not recursively collect errors. Keeping their counts separate avoids
        // inventing a factor of two at every nested union. Marking is the
        // independent unevaluated-annotation walk, which itself uses probes.
        #[derive(Clone, Copy, Default)]
        struct Passes {
            errors: usize,
            validity: usize,
            marking: usize,
        }
        impl Passes {
            fn total(self) -> usize {
                self.errors
                    .saturating_add(self.validity)
                    .saturating_add(self.marking)
            }
        }
        struct Task<'s, 'i> {
            schema: SchemaView<'s>,
            instance: NodeRef<'i>,
            passes: Passes,
            depth: usize,
            enclosing_error_units: usize,
        }
        let mut pending = vec![Task {
            schema: self,
            instance: NodeRef::new(node),
            passes: Passes {
                errors: 1,
                ..Passes::default()
            },
            depth: 0,
            enclosing_error_units: 0,
        }];
        let mut cost = 0usize;
        while let Some(task) = pending.pop() {
            let Task {
                schema,
                instance,
                passes,
                depth,
                enclosing_error_units,
            } = task;
            let (units, unique, immediate) = match instance {
                NodeRef::Value(node) | NodeRef::Dispatch { node, .. } => {
                    measured[&(node as *const ResolvedNode<'_> as usize)]
                }
                NodeRef::Property(name) => {
                    let units = 1usize.saturating_add(name.len().div_ceil(64));
                    (units, 0, units)
                }
            };
            let uses_uniqueness = schema.value().get("uniqueItems") == Some(&Value::Bool(true));
            let comparison = if uses_uniqueness { unique } else { 0 };
            // is_valid probes do not construct ValidationErrors or call
            // lazy_value. Keep deep data/comparison work for const/enum and
            // uniqueItems; primitive checks and structural routing use only
            // the immediate node. Errors/marking retain full subtree costs.
            let probe_units = if uses_uniqueness
                || schema.value().get("const").is_some()
                || schema.value().get("enum").is_some()
            {
                units
            } else {
                immediate
            };
            let data_work = units
                .max(enclosing_error_units)
                .saturating_add(comparison)
                .saturating_mul(passes.errors)
                .saturating_add(
                    units
                        .saturating_add(comparison)
                        .saturating_mul(passes.marking),
                )
                .saturating_add(
                    probe_units
                        .saturating_add(comparison)
                        .saturating_mul(passes.validity),
                );
            cost = cost.saturating_add(schema.entry().local_work.saturating_mul(data_work));
            if cost > limit || depth > MAX_SCHEMA_DEPTH {
                return limit.saturating_add(1);
            }
            // Charge before queue allocation. Weighted modes coalesce identical
            // structural traversals without expanding the engine's replay tree.
            let mut push = |schema, instance, passes: Passes, enclosing_error_units| {
                if passes.total() == 0 {
                    return;
                }
                cost = cost.saturating_add(1);
                if cost <= limit {
                    pending.push(Task {
                        schema,
                        instance,
                        passes,
                        depth: depth + 1,
                        enclosing_error_units,
                    });
                }
            };
            if passes.errors.saturating_add(passes.validity) != 0
                && ["unevaluatedProperties", "unevaluatedItems"]
                    .iter()
                    .any(|keyword| {
                        schema
                            .value()
                            .get(keyword)
                            .is_some_and(|value| value != &Value::Bool(true))
                    })
            {
                push(
                    schema,
                    instance,
                    Passes {
                        marking: passes.errors.saturating_add(passes.validity),
                        ..Passes::default()
                    },
                    enclosing_error_units,
                );
            }
            if let Some(reference) = schema.referenced() {
                push(reference, instance, passes, enclosing_error_units);
            }
            for child in profile::children(schema.value()).filter(|child| child.applies) {
                let view = schema
                    .registry
                    .locate(child.value)
                    .expect("indexed schema child");
                // Structural applicability only: do not evaluate a dependency
                // schema when its trigger member is absent (or not an object).
                if child.keyword() == "dependentSchemas"
                    && !matches!(
                        instance,
                        NodeRef::Value(ResolvedNode { value: ResolvedValue::Mapping(fields), .. })
                            if child.map_key().is_some_and(|key| fields.contains_key(key))
                    )
                {
                    continue;
                }
                if !child.descends() {
                    let child_passes = match child.keyword() {
                        "anyOf" | "oneOf" => Passes {
                            errors: passes.errors,
                            validity: passes
                                .validity
                                .saturating_add(passes.errors)
                                .saturating_add(passes.marking),
                            marking: passes.marking,
                        },
                        // Conditions and negation test validity; they never
                        // collect their failing condition's nested errors.
                        "if" => Passes {
                            validity: passes.total(),
                            marking: passes.marking,
                            ..Passes::default()
                        },
                        "not" => Passes {
                            validity: passes.errors.saturating_add(passes.validity),
                            ..Passes::default()
                        },
                        _ => Passes {
                            errors: passes.errors,
                            validity: passes.validity.saturating_add(passes.marking),
                            marking: passes.marking,
                        },
                    };
                    push(view, instance, child_passes, enclosing_error_units);
                    continue;
                }
                // contains and unevaluated validators probe item/property
                // validity and construct their own outer errors. Other
                // structural applicators preserve the parent's error phase.
                let child_passes = if matches!(
                    child.keyword(),
                    "contains" | "unevaluatedProperties" | "unevaluatedItems"
                ) {
                    Passes {
                        validity: passes.total(),
                        ..Passes::default()
                    }
                } else {
                    Passes {
                        errors: passes.errors,
                        validity: passes.validity.saturating_add(passes.marking),
                        ..Passes::default()
                    }
                };
                match instance {
                    NodeRef::Value(ResolvedNode {
                        value: ResolvedValue::Mapping(fields),
                        ..
                    }) => {
                        for (key, value) in fields {
                            let applies = match child.keyword() {
                                "properties" => child.map_key() == Some(key.as_ref()),
                                "additionalProperties"
                                    if schema.value().get("patternProperties").is_none() =>
                                {
                                    schema
                                        .value()
                                        .get("properties")
                                        .and_then(Value::as_object)
                                        .is_none_or(|properties| {
                                            !properties.contains_key(key.as_ref())
                                        })
                                }
                                "prefixItems" | "items" | "contains" | "unevaluatedItems"
                                | "additionalItems" => false,
                                _ => true,
                            };
                            if applies {
                                let instance = if child.keyword() == "propertyNames" {
                                    NodeRef::Property(key)
                                } else {
                                    NodeRef::new(value)
                                };
                                // Each collected property-name error wraps a
                                // deep copy of the containing object. Carry
                                // that cost through the key's error branches,
                                // without changing the key validation instance.
                                let error_units = if child.keyword() == "propertyNames" {
                                    units.max(enclosing_error_units)
                                } else {
                                    enclosing_error_units
                                };
                                push(view, instance, child_passes, error_units);
                            }
                        }
                    }
                    NodeRef::Value(ResolvedNode {
                        value: ResolvedValue::Sequence(items),
                        ..
                    }) => {
                        for (index, value) in items.iter().enumerate() {
                            let applies = match child.keyword() {
                                "prefixItems" => child.array_index() == Some(index),
                                "items" => {
                                    index
                                        >= schema
                                            .value()
                                            .get("prefixItems")
                                            .and_then(Value::as_array)
                                            .map_or(0, Vec::len)
                                }
                                "contains" | "unevaluatedItems" | "additionalItems" => true,
                                _ => false,
                            };
                            if applies {
                                push(
                                    view,
                                    NodeRef::new(value),
                                    child_passes,
                                    enclosing_error_units,
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
            if cost > limit {
                return limit.saturating_add(1);
            }
        }
        cost
    }
    pub(crate) fn validate(self, node: &ResolvedNode<'_>) -> Result<(), SchemaFailure> {
        let entry = self.entry();
        self.registry
            .validator
            .validate(NodeRef::Dispatch {
                key: &entry.dispatch_key,
                node,
            })
            .map_err(|error| {
                let schema_location = compiled_error_location(
                    &error,
                    &self.registry.entries,
                    &self.registry.compiled_resource,
                    &entry.diagnostic_location,
                );
                let mut path = error.instance_path().to_string();
                // The dispatcher has only properties constraints, so an error
                // must originate beneath this exact borrowed singleton field.
                path.drain(..entry.dispatch_key.len() + 1);
                SchemaFailure {
                    path,
                    schema: schema_location,
                    message: compiled_error_message(&error, &self.registry.entries, true),
                }
            })
    }
}
struct SchemaEntry {
    value: Value,
    location: SchemaLocation,
    // Canonical $id resource/pointer for diagnostics, distinct from the storage
    // identity used by bindings and default provenance.
    diagnostic_location: SchemaLocation,
    default_origin: Option<SchemaLocation>,
    default_tags: HashMap<usize, usize>,
    dispatch_key: String,
    work: Option<usize>,
    local_work: usize,
    has_defaults: bool,
    uses_uniqueness: bool,
}

/// Immutable, explicitly composed schema registry and its compiled validators.
pub struct Schemas {
    entries: Vec<SchemaEntry>,
    validator: Validator<DamlJson>,
    compiled_resource: String,
    by_type: HashMap<String, SchemaId>,
    by_binding: HashMap<String, SchemaId>,
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
        let mut stats = prepare_schema_graph(&registry_contents, &registry)?;
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
            append_schema_entries(binding.value, &binding.id, &mut entries, &mut total)?;
            by_type.insert(name, SchemaId(index));
        }
        for (uri, value) in resources {
            append_schema_entries(value, &uri, &mut entries, &mut total)?;
        }
        let by_binding = entries
            .iter()
            .enumerate()
            .map(|(id, entry)| {
                let binding = if entry.location.pointer.is_empty() {
                    entry.location.resource.clone()
                } else {
                    canonical_reference(&entry.location.resource, &entry.location.pointer)
                };
                (binding, SchemaId(id))
            })
            .collect();
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
        // All original URI/anchor context has already been resolved above.
        // Compile one internal resource: a constant base/scope makes static
        // reference target caching independent of the path used to reach it.
        let compiled_resource = (0usize..)
            .map(|index| format!("urn:daml:compiled:{index}"))
            .find(|uri| {
                let base = jsonschema::uri::from_str(uri).expect("generated absolute URI");
                registry.resolver(base).lookup("#").is_err()
            })
            .expect("a finite registry cannot occupy every generated URI");
        let mut definitions = Map::new();
        let mut properties = Map::new();
        let (mut compile_work, mut compiled_bytes) = check_value_limits(&serde_json::json!({
            "$id": compiled_resource,
            "$defs": {},
            "properties": {},
        }))?;
        compile_work = compile_work.saturating_add(compiled_bytes.div_ceil(64));
        for (id, entry) in entries.iter_mut().enumerate() {
            let mut value = entry.value.clone();
            let mut originals = profile::children(&entry.value);
            profile::for_each_child_mut(&mut value, |child| {
                let original = originals.next().expect("matching profile traversal");
                // Boolean schemas have keyword-specific compiler fast paths
                // and public error placement. Keep them literal; they remain
                // independently indexed/dispatchable like every other view.
                if original.value.is_boolean() {
                    return;
                }
                let target = by_address[&(original.value as *const Value as usize)];
                *child = serde_json::json!({"$ref": format!("#/$defs/{}", target.0)});
            });
            if let Some(object) = value.as_object_mut() {
                for keyword in [
                    "$id",
                    "$anchor",
                    "$dynamicAnchor",
                    "$recursiveAnchor",
                    "$defs",
                    "definitions",
                ] {
                    object.remove(keyword);
                }
                if let Some(target) = references.get(&SchemaId(id)) {
                    object.insert(
                        "$ref".into(),
                        Value::String(format!("#/$defs/{}", target.0)),
                    );
                }
            }
            // Only this flattened body's operative keywords execute here.
            // Defaults, annotations and child bodies are not re-evaluated at
            // every visit. Runtime error/marking phases still reserve full
            // subtrees for upstream eager error materialization.
            entry.local_work = value
                .as_object()
                .into_iter()
                .flat_map(|object| object.iter())
                .filter(|(keyword, value)| profile::validates(keyword, value))
                .try_fold(1usize, |work, (keyword, value)| {
                    let (nodes, bytes) = check_value_bounds(value, MAX_COMPILE_NODES)?;
                    Ok::<_, SchemaError>(
                        work.saturating_add(nodes)
                            .saturating_add(keyword.len().saturating_add(bytes).div_ceil(64)),
                    )
                })?;
            let wrapper = serde_json::json!({"$ref": format!("#/$defs/{id}")});
            let (nodes, bytes) = check_value_bounds(&value, MAX_COMPILE_NODES)?;
            let (wrapper_nodes, wrapper_bytes) = check_value_limits(&wrapper)?;
            // Ordinary target caching does not cover annotation construction:
            // helper caches contain only currently pending targets. Reserve
            // every completed-target revisit with its path multiplicity.
            let helper_roots = ["unevaluatedProperties", "unevaluatedItems"]
                .iter()
                .filter(|keyword| {
                    entry
                        .value
                        .get(**keyword)
                        .is_some_and(|value| value != &Value::Bool(true))
                })
                .count();
            let helper_work = stats
                .get(&entry.location)
                .expect("every compiled view was preflighted")
                .helper_work
                .saturating_mul(helper_roots);
            let member_bytes = bytes
                .saturating_add(wrapper_bytes)
                .saturating_add(entry.dispatch_key.len() * 2 + 6)
                .saturating_add(if id == 0 { 0 } else { 2 });
            compile_work = compile_work
                .saturating_add(nodes)
                .saturating_add(wrapper_nodes)
                .saturating_add(member_bytes.div_ceil(64))
                .saturating_add(helper_work);
            compiled_bytes = compiled_bytes.saturating_add(member_bytes);
            if compile_work > MAX_COMPILE_NODES || compiled_bytes > MAX_SOURCE_BYTES {
                return Err(limit(
                    "normalized schema compilation exceeds aggregate node/byte bounds",
                ));
            }
            definitions.insert(entry.dispatch_key.clone(), value);
            properties.insert(entry.dispatch_key.clone(), wrapper);
        }
        // Prepared diagnostic strings have one final owner; consume the
        // ephemeral statistics after the last compilation-budget lookup.
        for entry in &mut entries {
            let info = stats
                .remove(&entry.location)
                .expect("every compiled view was preflighted");
            entry.work = info.work;
            entry.diagnostic_location = info.diagnostic_location;
            entry.has_defaults = info.defaults;
            entry.uses_uniqueness = info.uniqueness;
        }
        let dispatcher = serde_json::json!({
            "$id": compiled_resource,
            "$defs": definitions,
            "properties": properties,
        });
        let validator = options.build(&dispatcher).map_err(|error| {
            let fallback = SchemaLocation {
                resource: String::new(),
                pointer: String::new(),
            };
            let location = native_regex_error_location(&error).unwrap_or_else(|| {
                compiled_error_location(&error, &entries, &compiled_resource, &fallback)
            });
            // Upstream paths cannot be rewritten on ValidationError. Translate
            // this expected schema failure here rather than expose the internal
            // compiler namespace through a downcastable error source.
            SchemaError::new(
                SchemaErrorKind::InvalidSchema,
                format!("schema compilation failed: {error}"),
            )
            .at(location)
        })?;
        Ok(Self {
            entries,
            validator,
            compiled_resource,
            by_type,
            by_binding,
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
                    let Some(binding_id) =
                        self.semantic_binding_schema(binding).map(SchemaView::id)
                    else {
                        return Err(crate::semantic::unrepresentable_tag(path));
                    };
                    if tag.name.len() <= crate::semantic::MAX_SCALAR_BYTES
                        && visible(&tag.name)
                        && self.binding_identity(&tag.name) == Some(binding_id)
                    {
                        return Ok(tag.name.as_str());
                    }
                    self.by_type
                        .keys()
                        .filter(|name| {
                            name.len() <= crate::semantic::MAX_SCALAR_BYTES
                                && visible(name)
                                && self.binding_identity(name) == Some(binding_id)
                        })
                        .min()
                        .map(String::as_str)
                        .ok_or_else(|| crate::semantic::unrepresentable_tag(path))
                }
                None => {
                    if tag.name.len() > crate::semantic::MAX_SCALAR_BYTES {
                        Err(crate::semantic::tag_limit_error(path))
                    } else if crate::names::valid_tag(&tag.name) && visible(&tag.name) {
                        Ok(tag.name.as_str())
                    } else {
                        Err(crate::semantic::unrepresentable_tag(path))
                    }
                }
            }
        })
    }
    pub(crate) fn binding_identity(&self, name: &str) -> Option<SchemaId> {
        let mut current = self.binding_schema(name)?.id();
        let mut steps = 0;
        while let Some(next) = self.alias_target(current) {
            if steps >= self.entries.len() {
                return None;
            }
            current = next;
            steps += 1;
        }
        Some(current)
    }

    fn alias_target(&self, id: SchemaId) -> Option<SchemaId> {
        let next = self.references.get(&id).copied()?;
        let entry = &self.entries[id.0];
        let pure_alias = entry.value.as_object().is_some_and(|object| {
            object.get("$ref").and_then(Value::as_str).is_some()
                && object.keys().all(|key| key == "$ref" || key == "$id")
                && (object.len() == 1
                    || (object.len() == 2
                        && object.get("$id").and_then(Value::as_str)
                            == Some(entry.location.resource.as_str())))
        });
        pure_alias.then_some(next)
    }

    pub(crate) fn type_schema(&self, tag: &str) -> Option<SchemaView<'_>> {
        self.by_type.get(tag).copied().map(|id| self.view(id))
    }
    pub(crate) fn binding_schema(&self, binding: &str) -> Option<SchemaView<'_>> {
        self.by_binding
            .get(binding)
            .copied()
            .map(|id| self.view(id))
            .or_else(|| self.type_schema(binding))
    }
    pub(crate) fn semantic_binding_schema(&self, binding: &str) -> Option<SchemaView<'_>> {
        let schema = self.binding_schema(binding)?;
        // An opaque supplied identity must name its actual contract, not a local
        // alias wrapper whose target could differ in another registry.
        self.alias_target(schema.id()).is_none().then_some(schema)
    }

    pub(crate) fn canonical_binding_schema(&self, binding: &str) -> Option<SchemaView<'_>> {
        self.binding_identity(binding).map(|id| self.view(id))
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
    entries: &mut Vec<SchemaEntry>,
    total: &mut usize,
) -> Result<(), SchemaError> {
    fn visit(
        value: &Value,
        resource: &str,
        pointer: &str,
        entries: &mut Vec<SchemaEntry>,
        total: &mut usize,
    ) -> Result<(), SchemaError> {
        *total = total.saturating_add(count_nodes(value));
        if *total > MAX_COMPILE_NODES {
            return Err(limit("aggregate schema compilation work exceeds bound"));
        }
        let location = SchemaLocation {
            resource: resource.into(),
            pointer: pointer.into(),
        };
        let default_origin = value.get("default").map(|_| SchemaLocation {
            resource: resource.into(),
            pointer: crate::pointer::join(pointer, "default"),
        });
        entries.push(SchemaEntry {
            value: value.clone(),
            location,
            diagnostic_location: SchemaLocation {
                resource: String::new(),
                pointer: String::new(),
            },
            default_origin,
            default_tags: HashMap::new(),
            dispatch_key: entries.len().to_string(),
            work: None,
            local_work: 0,
            has_defaults: false,
            uses_uniqueness: false,
        });
        for child in profile::children(value) {
            visit(
                child.value,
                resource,
                &child.pointer(pointer),
                entries,
                total,
            )?;
        }
        Ok(())
    }
    let mut nodes = 0;
    walk_schema(&value, 0, &mut nodes)?;
    visit(&value, resource, "", entries, total)
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
    check_value_bounds(value, MAX_SCHEMA_NODES)
}

// Source admission and ephemeral compiler representation share exact accounting;
// only the latter is governed by the existing compilation-node bound.
fn check_value_bounds(value: &Value, node_bound: usize) -> Result<(usize, usize), SchemaError> {
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
        node_bound: usize,
    ) -> Result<(), SchemaError> {
        *nodes = nodes.saturating_add(1);
        if *nodes > node_bound || depth > MAX_SCHEMA_DEPTH {
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
                    visit(value, depth + 1, nodes, bytes, node_bound)?
                }
            }
            Value::Object(values) => {
                *bytes = bytes.saturating_add(2 + values.len().saturating_sub(1));
                for (key, value) in values {
                    *bytes = bytes.saturating_add(string_size(key) + 1);
                    visit(value, depth + 1, nodes, bytes, node_bound)?;
                }
            }
        }
        if *bytes > MAX_SOURCE_BYTES {
            return Err(limit("schema module exceeds 8 MiB"));
        }
        Ok(())
    }
    let (mut nodes, mut bytes) = (0, 0);
    visit(value, 0, &mut nodes, &mut bytes, node_bound)?;
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
struct SchemaStats {
    // None denotes a graph whose evaluation needs an actual, finite instance.
    work: Option<usize>,
    helper_work: usize,
    defaults: bool,
    uniqueness: bool,
    diagnostic_location: SchemaLocation,
}

/// Admit references against the engine's prepared resources, before compilation.
/// Definition containment contributes compilation work but never evaluation or
/// instance progress. Only cycles in the same-instance evaluation graph are
/// forbidden; structural cycles reserve their work against each actual instance.
fn prepare_schema_graph(
    resources: &HashMap<String, Value>,
    registry: &Registry<'_>,
) -> Result<HashMap<SchemaLocation, SchemaStats>, SchemaError> {
    fn collect(
        value: &Value,
        resource: &str,
        pointer: &str,
        base: &jsonschema::Uri<String>,
        resource_pointer: &str,
        locations: &mut Vec<(SchemaLocation, SchemaLocation)>,
    ) -> Result<(), SchemaError> {
        let scoped;
        let (base, resource_pointer) = if let Some(id) = value.get("$id").and_then(Value::as_str) {
            scoped = jsonschema::uri::resolve_against(&base.borrow(), id)
                .map_err(|error| invalid(format!("invalid schema identifier {id:?}: {error}")))?;
            (&scoped, pointer)
        } else {
            (base, resource_pointer)
        };
        locations.push((
            SchemaLocation {
                resource: resource.into(),
                pointer: pointer.into(),
            },
            SchemaLocation {
                resource: base.as_str().to_owned(),
                pointer: pointer
                    .strip_prefix(resource_pointer)
                    .expect("a schema view is within its containing resource")
                    .to_owned(),
            },
        ));
        for child in profile::children(value) {
            collect(
                child.value,
                resource,
                &child.pointer(pointer),
                base,
                resource_pointer,
                locations,
            )?;
        }
        Ok(())
    }
    struct Edge {
        target: usize,
        restricted: bool,
        applies: bool,
        descends: bool,
    }
    struct Prepared<'a> {
        value: &'a Value,
        base: String,
        edges: Vec<Edge>,
        location: SchemaLocation,
        diagnostic_location: SchemaLocation,
        own_work: usize,
    }
    let mut locations = Vec::new();
    let (mut total_nodes, mut total_bytes) = (0usize, 0usize);
    for (uri, schema) in resources {
        let (nodes, bytes) = check_value_limits(schema)?;
        total_nodes = total_nodes.saturating_add(nodes);
        total_bytes = total_bytes.saturating_add(bytes);
        let base = jsonschema::uri::from_str(uri)
            .map_err(|error| invalid(format!("invalid schema URI: {error}")))?;
        collect(schema, uri, "", &base, "", &mut locations)?;
    }
    if total_nodes > MAX_SCHEMA_NODES || total_bytes > MAX_SOURCE_BYTES {
        return Err(limit("composed schemas exceed aggregate node/byte bounds"));
    }
    let mut prepared = Vec::new();
    let mut addresses = HashMap::new();
    let mut copied_bytes = 0usize;
    for (location, diagnostic_location) in locations {
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
            diagnostic_location,
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
            edges.push(Edge {
                target,
                restricted: false,
                applies: true,
                descends: false,
            });
        }
        for child in profile::children(node.value) {
            let target = addresses
                .get(&(child.value as *const Value as usize))
                .copied()
                .ok_or_else(|| invalid("schema child is absent from the prepared registry"))?;
            edges.push(Edge {
                target,
                restricted: child.unsupported_defaults,
                applies: child.applies,
                descends: child.descends(),
            });
        }
        node.edges = edges;
    }
    // Explicit DFS frames also bound admission's own stack on hostile graphs.
    fn order(
        nodes: &[Prepared<'_>],
        same_instance: bool,
        recursive: &mut [bool],
    ) -> Result<Vec<usize>, SchemaError> {
        let mut state = vec![0u8; nodes.len()];
        let mut finished = Vec::with_capacity(nodes.len());
        for root in 0..nodes.len() {
            if state[root] != 0 {
                continue;
            }
            state[root] = 1;
            let mut stack = vec![(root, 0usize)];
            while let Some((id, cursor)) = stack.last_mut() {
                let id = *id;
                let Some(edge) = nodes[id].edges.get(*cursor) else {
                    state[id] = 2;
                    finished.push(id);
                    stack.pop();
                    continue;
                };
                *cursor += 1;
                if same_instance && (!edge.applies || edge.descends) {
                    continue;
                }
                match state[edge.target] {
                    1 if same_instance => {
                        return Err(SchemaError::new(
                            SchemaErrorKind::ReferenceCycle,
                            "schema reference cycle without instance descent",
                        )
                        .at(nodes[edge.target].location.clone()));
                    }
                    1 => recursive[edge.target] = true,
                    0 => {
                        if stack.len() >= MAX_SCHEMA_DEPTH {
                            return Err(limit("expanded schema graph exceeds depth bound"));
                        }
                        state[edge.target] = 1;
                        stack.push((edge.target, 0));
                    }
                    _ => {}
                }
            }
        }
        Ok(finished)
    }
    let mut recursive = vec![false; prepared.len()];
    let same_instance = order(&prepared, true, &mut recursive)?;
    let finished = order(&prepared, false, &mut recursive)?;
    let mut parents = vec![Vec::new(); prepared.len()];
    for (id, node) in prepared.iter().enumerate() {
        for edge in &node.edges {
            parents[edge.target].push((id, edge.applies));
        }
    }
    let mut stats: Vec<_> = prepared
        .iter()
        .map(|node| SchemaStats {
            work: None,
            helper_work: 0,
            defaults: node.value.get("default").is_some(),
            uniqueness: node.value.get("uniqueItems") == Some(&Value::Bool(true)),
            diagnostic_location: SchemaLocation {
                resource: String::new(),
                pointer: String::new(),
            },
        })
        .collect();
    // The engine's unevaluated helper caches are pending-only: siblings rebuild
    // completed targets. Helpers follow same-instance applicators/references,
    // not properties/items or definition containment. That graph is a DAG by
    // admission, so this DP retains every edge's multiplicity without expanding
    // the potentially exponential helper tree in our own preflight.
    let mut same_instance_heights = vec![1usize; prepared.len()];
    for id in same_instance {
        let node = &prepared[id];
        // References, when present, are the first prepared edge; only physical
        // object children acquire extra wrappers. Booleans remain literal.
        let inline_wrappers = node
            .edges
            .iter()
            .skip(usize::from(
                node.value.get("$ref").and_then(Value::as_str).is_some(),
            ))
            .filter(|edge| edge.applies && prepared[edge.target].value.is_object())
            .count();
        // A canonical child wrapper has two JSON values and less than 64 bytes
        // (view ids are bounded by MAX_SCHEMA_NODES). Reserve its three work
        // units twice: once for helper construction and once for any ordinary
        // compile invoked at the helper's potentially different location.
        let mut work = node
            .own_work
            .saturating_add(inline_wrappers.saturating_mul(6));
        for edge in node
            .edges
            .iter()
            .filter(|edge| edge.applies && !edge.descends)
        {
            work = work.saturating_add(stats[edge.target].helper_work);
            same_instance_heights[id] =
                same_instance_heights[id].max(same_instance_heights[edge.target].saturating_add(1));
        }
        if same_instance_heights[id] > MAX_SCHEMA_DEPTH {
            return Err(limit(
                "expanded same-instance schema graph exceeds depth bound",
            ));
        }
        stats[id].helper_work = work.min(MAX_COMPILE_NODES + 1);
    }
    // Every flag changes at most once. Propagate to a fixed point across SCCs,
    // including defaults hidden behind refs and restricted applicators.
    let mut pending: Vec<_> = (0..prepared.len()).collect();
    while let Some(id) = pending.pop() {
        for &(parent, applies) in &parents[id] {
            let defaults = stats[parent].defaults | (applies && stats[id].defaults);
            let uniqueness = stats[parent].uniqueness | (applies && stats[id].uniqueness);
            let cyclic = recursive[parent] | recursive[id];
            if defaults != stats[parent].defaults
                || uniqueness != stats[parent].uniqueness
                || cyclic != recursive[parent]
            {
                stats[parent].defaults = defaults;
                stats[parent].uniqueness = uniqueness;
                recursive[parent] = cyclic;
                pending.push(parent);
            }
        }
    }
    // Preserve admission compatibility for the old wholly acyclic profile.
    // In a recursive/mixed registry these expanded per-view trees are runtime
    // coefficients, not compiler work: compilation now has one normalized owner
    // whose actual representation/shared-target work is reserved in compose.
    let legacy_acyclic = !recursive.iter().any(|recursive| *recursive);
    let mut heights = vec![1usize; prepared.len()];
    let mut total_work = 0usize;
    for id in finished {
        let mut work = prepared[id].own_work;
        if !recursive[id] {
            for edge in &prepared[id].edges {
                work = work.saturating_add(
                    stats[edge.target]
                        .work
                        .expect("acyclic children precede parents"),
                );
                heights[id] = heights[id].max(heights[edge.target].saturating_add(1));
            }
            if heights[id] > MAX_SCHEMA_DEPTH {
                return Err(limit("expanded schema graph exceeds depth bound"));
            }
            stats[id].work = Some(work);
        }
        if legacy_acyclic {
            total_work = total_work.saturating_add(work);
            if total_work > MAX_COMPILE_NODES {
                return Err(limit("aggregate expanded compilation work exceeds bound"));
            }
        }
    }
    let mut reachable = HashSet::new();
    let mut pending: Vec<_> = prepared
        .iter()
        .enumerate()
        .filter(|(_, node)| node.location.pointer.is_empty())
        .map(|(id, _)| id)
        .collect();
    while let Some(id) = pending.pop() {
        if !reachable.insert(id) {
            continue;
        }
        for edge in prepared[id].edges.iter().filter(|edge| edge.applies) {
            if edge.restricted && stats[edge.target].defaults {
                return Err(SchemaError::new(
                    SchemaErrorKind::UnsupportedDefaults,
                    "defaults reachable through an unsupported applicator are unsupported",
                )
                .at(prepared[edge.target].location.clone()));
            }
            pending.push(edge.target);
        }
    }
    Ok(prepared
        .into_iter()
        .zip(stats)
        .map(|(node, mut info)| {
            info.diagnostic_location = node.diagnostic_location;
            (node.location, info)
        })
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

fn compiled_error_message(
    error: &ValidationError<'_>,
    entries: &[SchemaEntry],
    masked: bool,
) -> String {
    match error.kind() {
        ValidationErrorKind::Not { schema } => {
            // The engine captures the normalized not-child for diagnostics.
            // Render its authored view, not the private dispatch reference.
            let authored = match schema
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
            {
                Some(key) => {
                    &entries[key.parse::<usize>().expect("generated compiled view key")].value
                }
                None => schema,
            };
            if masked {
                format!("{authored} is not allowed for value")
            } else {
                format!("{authored} is not allowed for {}", error.instance())
            }
        }
        // Match the engine's property-name rendering: it displays the nested
        // key error rather than applying the outer instance placeholder.
        ValidationErrorKind::PropertyNames { error } => {
            compiled_error_message(error, entries, false)
        }
        _ if masked => error.masked().to_string(),
        _ => error.to_string(),
    }
}

fn compiled_error_location(
    error: &ValidationError<'_>,
    entries: &[SchemaEntry],
    resource: &str,
    fallback: &SchemaLocation,
) -> SchemaLocation {
    let location = validation_error_location(error, fallback);
    if error.absolute_keyword_location().is_some() && location.resource != resource {
        return location;
    }
    let Some(path) = location.pointer.strip_prefix("/$defs/") else {
        return fallback.clone();
    };
    let (key, suffix) = path.split_once('/').unwrap_or((path, ""));
    let entry = &entries[key.parse::<usize>().expect("generated compiled view key")];
    SchemaLocation {
        resource: entry.diagnostic_location.resource.clone(),
        pointer: if suffix.is_empty() {
            entry.diagnostic_location.pointer.clone()
        } else {
            format!("{}/{suffix}", entry.diagnostic_location.pointer)
        },
    }
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
