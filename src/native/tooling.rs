//! Source-backed editor views of the native compiler, never data admission.

pub(crate) use super::bundle::editor_watch_paths;

use super::{Bundle, Definition, Expression, ExpressionKind, NativeError, NativeErrorKind};
use crate::{Document, SchemaLocation, Schemas, Span, Value};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
};

pub(crate) struct EditorCompilation {
    pub(crate) schemas: Result<Schemas, NativeError>,
    pub(crate) index: EditorIndex,
}

#[derive(Debug)]
pub(crate) struct EditorLocation<'a> {
    pub(crate) path: &'a Path,
    pub(crate) source: &'a str,
    pub(crate) span: Span,
}

#[derive(Default)]
pub(super) struct EditorBudget {
    nodes: usize,
    bytes: usize,
}

impl EditorBudget {
    pub(super) fn charge(
        &mut self,
        nodes: usize,
        bytes: usize,
        path: &Path,
    ) -> Result<(), NativeError> {
        let nodes = self.nodes.saturating_add(nodes);
        let bytes = self.bytes.saturating_add(bytes);
        if nodes > super::bundle::MAX_SOURCE_TABLE_ENTRIES
            || bytes > super::bundle::MAX_SOURCE_TABLE_BYTES
        {
            return Err(NativeError::new(
                NativeErrorKind::LimitExceeded,
                "native editor metadata exceeds node/source-byte bounds",
            )
            .at_path(path));
        }
        self.nodes = nodes;
        self.bytes = bytes;
        Ok(())
    }

    pub(super) fn remaining_bytes(&self) -> usize {
        super::bundle::MAX_SOURCE_TABLE_BYTES.saturating_sub(self.bytes)
    }
}

enum SourceContent {
    Definition(Rc<Definition>),
    Text(String),
}

struct EditorSource {
    content: SourceContent,
    root: Option<PathBuf>,
}

impl EditorSource {
    fn text(&self) -> &str {
        match &self.content {
            SourceContent::Definition(definition) => definition.source(),
            SourceContent::Text(source) => source,
        }
    }
}

#[derive(Default)]
pub(crate) struct EditorIndex {
    budget: EditorBudget,
    entry: PathBuf,
    dependencies: Vec<PathBuf>,
    pending_dependencies: HashSet<PathBuf>,
    sources: HashMap<PathBuf, EditorSource>,
    source_aliases: HashMap<PathBuf, PathBuf>,
    references: HashMap<PathBuf, HashMap<String, PathBuf>>,
    origins: HashMap<String, super::compiler::SchemaOrigin>,
    aliases: HashMap<String, String>,
}

impl Bundle {
    pub(crate) fn compile_editor(
        &self,
        document: Option<&Document>,
        expected: Option<&str>,
        additional_types: &[&str],
    ) -> EditorCompilation {
        super::compiler::compile_editor(self, document, expected, additional_types)
    }
}

impl EditorIndex {
    pub(super) fn initialize(&mut self, bundle: &Bundle) -> Result<(), NativeError> {
        self.budget
            .charge(1, bundle.root.as_os_str().len(), &bundle.root)?;
        self.entry = bundle.root.clone();
        let manifest = bundle.root.join(super::BUNDLE_MANIFEST);
        self.dependency(&manifest)?;
        let actual = bundle.editor_source_path(&manifest)?;
        self.dependency(&actual)?;
        self.source_alias(&manifest, &actual)?;
        let source = bundle.read_resource(&manifest, self.budget.remaining_bytes())?;
        self.retain_text(&actual, source)
    }

    pub(super) fn source_alias(&mut self, path: &Path, actual: &Path) -> Result<(), NativeError> {
        if path != actual && !self.source_aliases.contains_key(path) {
            self.budget.charge(
                1,
                path.as_os_str()
                    .len()
                    .saturating_add(actual.as_os_str().len()),
                path,
            )?;
            self.source_aliases
                .insert(path.to_path_buf(), actual.to_path_buf());
        }
        Ok(())
    }

    pub(super) fn remaining_source_bytes(&self) -> usize {
        self.budget.remaining_bytes()
    }

    pub(super) fn dependency(&mut self, path: &Path) -> Result<(), NativeError> {
        if !self.pending_dependencies.contains(path) {
            self.budget.charge(1, path.as_os_str().len(), path)?;
            self.pending_dependencies.insert(path.to_path_buf());
        }
        Ok(())
    }

    pub(super) fn retain_text(&mut self, path: &Path, source: String) -> Result<(), NativeError> {
        if self.sources.contains_key(path) {
            return Ok(());
        }
        self.budget
            .charge(1, path.as_os_str().len().saturating_add(source.len()), path)?;
        self.sources.insert(
            path.to_path_buf(),
            EditorSource {
                content: SourceContent::Text(source),
                root: None,
            },
        );
        Ok(())
    }

    pub(super) fn definition(
        &mut self,
        path: &Path,
        root: &Path,
        source: String,
    ) -> Result<Rc<Definition>, NativeError> {
        self.budget.charge(
            1,
            source
                .len()
                .saturating_add(path.as_os_str().len())
                .saturating_add(root.as_os_str().len()),
            path,
        )?;
        // Parse the same owned source through the existing parser, retaining it
        // even on failure. Successful ASTs are shared, not cloned or re-parsed.
        match crate::parser::native::parse_definition(&source, crate::Limits::default()) {
            Ok(expression) => {
                let definition = Rc::new(Definition {
                    source,
                    root: expression,
                });
                self.sources.insert(
                    path.to_path_buf(),
                    EditorSource {
                        content: SourceContent::Definition(Rc::clone(&definition)),
                        root: Some(root.to_path_buf()),
                    },
                );
                Ok(definition)
            }
            Err(error) => {
                self.sources.insert(
                    path.to_path_buf(),
                    EditorSource {
                        content: SourceContent::Text(source),
                        root: Some(root.to_path_buf()),
                    },
                );
                Err(
                    NativeError::new(NativeErrorKind::InvalidDefinition, error.to_string())
                        .at_path(path)
                        .at_span(error.span)
                        .caused_by(error),
                )
            }
        }
    }

    pub(super) fn reference(
        &mut self,
        root: &Path,
        tag: &str,
        path: &Path,
    ) -> Result<(), NativeError> {
        // Repeated recursive edges retain one target in their declaring scope.
        if self
            .references
            .get(root)
            .is_some_and(|names| names.contains_key(tag))
        {
            return Ok(());
        }
        let scope_bytes = if self.references.contains_key(root) {
            0
        } else {
            root.as_os_str().len()
        };
        self.budget.charge(
            1,
            scope_bytes
                .saturating_add(tag.len())
                .saturating_add(path.as_os_str().len()),
            path,
        )?;
        if let Some(names) = self.references.get_mut(root) {
            names.insert(tag.to_owned(), path.to_path_buf());
        } else {
            self.references.insert(
                root.to_path_buf(),
                HashMap::from([(tag.to_owned(), path.to_path_buf())]),
            );
        }
        Ok(())
    }

    pub(super) fn alias(
        &mut self,
        id: &str,
        original: &str,
        file: &Path,
    ) -> Result<(), NativeError> {
        self.budget
            .charge(1, id.len().saturating_add(original.len()), file)?;
        self.aliases.insert(id.to_owned(), original.to_owned());
        Ok(())
    }

    fn origin(&self, resource: &str) -> Option<&super::compiler::SchemaOrigin> {
        let Some(original) = self.aliases.get(resource) else {
            return self.origins.get(resource);
        };
        let mut resource = original.as_str();
        for _ in 0..crate::schema::profile::MAX_SCHEMA_DEPTH {
            if let Some(original) = self.aliases.get(resource) {
                resource = original;
            } else {
                return self.origins.get(resource);
            }
        }
        None
    }

    pub(super) fn finish(
        &mut self,
        origins: HashMap<String, super::compiler::SchemaOrigin>,
    ) -> Result<(), NativeError> {
        self.dependencies = self.pending_dependencies.drain().collect();
        self.dependencies.sort();
        let bytes = origins.iter().fold(0usize, |total, (name, origin)| {
            total
                .saturating_add(name.len())
                .saturating_add(origin.path.as_os_str().len())
        });
        self.budget.charge(origins.len(), bytes, &self.entry)?;
        // The compiler already owns this map; transferring it copies no IDs,
        // paths, source text or syntax trees into an editor-only mirror.
        self.origins = origins;
        Ok(())
    }

    pub(crate) fn dependencies(&self) -> &[PathBuf] {
        &self.dependencies
    }

    pub(crate) fn loaded_names(&self) -> impl Iterator<Item = &str> {
        self.references
            .get(&self.entry)
            .into_iter()
            .flat_map(|names| names.iter())
            .filter_map(move |(name, target)| {
                self.sources
                    .get(target)
                    .filter(|source| matches!(&source.content, SourceContent::Definition(_)))
                    .map(|_| name.as_str())
            })
    }

    fn source_entry(&self, path: &Path) -> Option<(&Path, &EditorSource)> {
        self.sources
            .get_key_value(path)
            .or_else(|| {
                let canonical = self.source_aliases.get(path)?;
                self.sources.get_key_value(canonical)
            })
            .map(|(path, source)| (path.as_path(), source))
    }

    pub(crate) fn source(&self, path: &Path) -> Option<&str> {
        self.source_entry(path).map(|(_, source)| source.text())
    }

    pub(crate) fn locations(&self, location: &SchemaLocation) -> Vec<EditorLocation<'_>> {
        if location.pointer.len() > super::bundle::MAX_SOURCE_TABLE_BYTES
            || location
                .pointer
                .bytes()
                .filter(|byte| *byte == b'/')
                .count()
                > 2 * crate::schema::profile::MAX_SCHEMA_DEPTH
        {
            return Vec::new();
        }
        let Some(origin) = self.origin(&location.resource) else {
            return Vec::new();
        };
        let Some((path, source)) = self.source_entry(&origin.path) else {
            return Vec::new();
        };
        let span = match &source.content {
            SourceContent::Definition(definition) => {
                super::compiler::expression_at(&definition.root, origin.span)
                    .map(|expression| {
                        expression_location(expression, definition.source(), &location.pointer)
                    })
                    .unwrap_or(origin.span)
            }
            SourceContent::Text(_) => origin.span,
        };
        vec![EditorLocation {
            path,
            source: source.text(),
            span,
        }]
    }

    pub(crate) fn reference_locations(
        &self,
        from: Option<&Path>,
        tag: &str,
    ) -> Vec<EditorLocation<'_>> {
        let root = from
            .and_then(|path| self.source_entry(path))
            .and_then(|(_, source)| source.root.as_deref())
            .unwrap_or(&self.entry);
        let Some(target) = self.references.get(root).and_then(|names| names.get(tag)) else {
            return Vec::new();
        };
        let Some((path, source)) = self.source_entry(target) else {
            return Vec::new();
        };
        let SourceContent::Definition(definition) = &source.content else {
            return Vec::new();
        };
        vec![EditorLocation {
            path,
            source: definition.source(),
            span: definition.span(),
        }]
    }
}

fn pointer_head(pointer: &str) -> Option<(&str, &str)> {
    let pointer = pointer.strip_prefix('/')?;
    Some(pointer.find('/').map_or((pointer, ""), |index| {
        (&pointer[..index], &pointer[index..])
    }))
}

// Project the native AST's actual lowering layout. This inspects source syntax,
// not schema validity, reference identity encodings, or application values.
fn expression_location(expression: &Expression, source: &str, pointer: &str) -> Span {
    let Some((keyword, rest)) = pointer_head(pointer) else {
        return expression.span;
    };
    if keyword == "default"
        && let super::DefaultValue::Value(node) = &expression.default
    {
        return value_location(node, source, rest, false);
    }
    match &expression.kind {
        ExpressionKind::Group(inner) | ExpressionKind::Closed(inner) => {
            expression_location(inner, source, pointer)
        }
        ExpressionKind::Record(members) if keyword == "properties" => {
            let Some((key, rest)) = pointer_head(rest) else {
                return expression.span;
            };
            let Some(key) = crate::pointer::decode_segment(key) else {
                return expression.span;
            };
            let Some(member) = members
                .iter()
                .find(|member| source.get(member.key.start..member.key.end) == Some(key.as_ref()))
            else {
                return expression.span;
            };
            if rest.is_empty() {
                member.key
            } else {
                expression_location(&member.constraint, source, rest)
            }
        }
        ExpressionKind::Tuple(items) if keyword == "prefixItems" => {
            indexed_expression(items, source, rest).unwrap_or(expression.span)
        }
        ExpressionKind::Union(items) if keyword == "anyOf" => {
            indexed_expression(items, source, rest).unwrap_or(expression.span)
        }
        ExpressionKind::Intersection(items) if keyword == "allOf" => {
            indexed_expression(items, source, rest).unwrap_or(expression.span)
        }
        ExpressionKind::Tagged {
            name,
            body: Some(body),
        } => {
            let tag = source.get(name.start..name.end).unwrap_or("");
            if tag == "Tuple" {
                return expression_location(body, source, pointer);
            }
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
                if let ExpressionKind::Record(members) = &body.kind {
                    let field = match keyword {
                        "items" => "items",
                        "additionalProperties" => "values",
                        "minimum" | "minLength" | "minItems" | "minProperties" => "min",
                        "maximum" | "maxLength" | "maxItems" | "maxProperties" => "max",
                        "exclusiveMinimum" => "exclusiveMin",
                        "exclusiveMaximum" => "exclusiveMax",
                        "multipleOf" => "multipleOf",
                        "x-daml-regex" => "pattern",
                        "uniqueItems" => "unique",
                        "x-daml-scalar" => "offset",
                        _ => return expression.span,
                    };
                    if let Some(member) = members
                        .iter()
                        .find(|member| source.get(member.key.start..member.key.end) == Some(field))
                    {
                        return if rest.is_empty() {
                            member.key
                        } else {
                            expression_location(&member.constraint, source, rest)
                        };
                    }
                }
            } else if keyword == "allOf"
                && let Some(("0", rest)) = pointer_head(rest)
            {
                return expression_location(body, source, rest);
            }
            expression.span
        }
        ExpressionKind::Literal(node) => value_location(node, source, pointer, true),
        _ => expression.span,
    }
}

fn indexed_expression(items: &[Expression], source: &str, pointer: &str) -> Option<Span> {
    let (index, rest) = pointer_head(pointer)?;
    let item = items.get(index.parse::<usize>().ok()?)?;
    Some(expression_location(item, source, rest))
}

fn value_location(node: &crate::Node, source: &str, pointer: &str, schema: bool) -> Span {
    let Some((first, rest)) = pointer_head(pointer) else {
        return node.span;
    };
    let (key, rest) = if schema {
        if !matches!(first, "properties" | "prefixItems") {
            return node.span;
        }
        let Some(parts) = pointer_head(rest) else {
            return node.span;
        };
        parts
    } else {
        (first, rest)
    };
    match &node.value {
        Value::Mapping(entries) => {
            let Some(key) = crate::pointer::decode_segment(key) else {
                return node.span;
            };
            let Some(entry) = entries
                .iter()
                .find(|entry| source.get(entry.key.start..entry.key.end) == Some(key.as_ref()))
            else {
                return node.span;
            };
            if rest.is_empty() {
                entry.key
            } else {
                value_location(&entry.value, source, rest, schema)
            }
        }
        Value::Sequence(items) => key
            .parse::<usize>()
            .ok()
            .and_then(|index| items.get(index))
            .map_or(node.span, |item| value_location(item, source, rest, schema)),
        _ => node.span,
    }
}
