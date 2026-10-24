use std::collections::BTreeMap;

use daml::{
    Document, Node, Origin, ResolvedNode, ResolvedValue, SemanticNode, SemanticTag, SemanticValue,
    Value,
};
use js_sys::{Array, JsString, Object, Reflect, WeakSet};
use wasm_bindgen::{JsCast, prelude::*};

use crate::diagnostic::{self, set};

pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_SCALAR: usize = 1024 * 1024;
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 128;

#[derive(Clone, Copy)]
enum Segment<'a> {
    Root,
    Key(&'a str),
    Index(u32),
}

pub(crate) struct DataPath<'a> {
    parent: Option<&'a DataPath<'a>>,
    segment: Segment<'a>,
    len: usize,
}

impl DataPath<'_> {
    pub(crate) fn root() -> Self {
        Self {
            parent: None,
            segment: Segment::Root,
            len: 0,
        }
    }

    fn key<'a>(&'a self, key: &'a str) -> DataPath<'a> {
        let escaped = key
            .bytes()
            .filter(|value| matches!(value, b'/' | b'~'))
            .count();
        DataPath {
            parent: Some(self),
            segment: Segment::Key(key),
            len: self.len + key.len() + escaped + 1,
        }
    }

    fn index(&self, index: u32) -> DataPath<'_> {
        let digits = if index == 0 {
            1
        } else {
            index.ilog10() as usize + 1
        };
        DataPath {
            parent: Some(self),
            segment: Segment::Index(index),
            len: self.len + digits + 1,
        }
    }

    fn pointer(&self) -> String {
        let mut segments = Vec::new();
        let mut current = Some(self);
        while let Some(path) = current {
            segments.push(path.segment);
            current = path.parent;
        }
        let mut out = String::with_capacity(self.len);
        for part in segments.into_iter().rev() {
            match part {
                Segment::Root => (),
                Segment::Index(value) => {
                    out.push('/');
                    out.push_str(&value.to_string());
                }
                Segment::Key(value) => {
                    out.push('/');
                    for value in value.chars() {
                        match value {
                            '~' => out.push_str("~0"),
                            '/' => out.push_str("~1"),
                            _ => out.push(value),
                        }
                    }
                }
            }
        }
        out
    }
}

fn invalid(kind: &str, message: &str, path: &DataPath<'_>) -> JsValue {
    diagnostic::error("semantic", kind, message, Some(&path.pointer()))
}

pub(crate) fn string(
    value: &JsValue,
    limit: usize,
    path: &DataPath<'_>,
) -> Result<String, JsValue> {
    string_for(value, limit, path, "semantic")
}

pub(crate) fn resource_string(
    value: &JsValue,
    limit: usize,
    path: &DataPath<'_>,
) -> Result<String, JsValue> {
    string_for(value, limit, path, "native")
}

fn string_for(
    value: &JsValue,
    limit: usize,
    path: &DataPath<'_>,
    category: &str,
) -> Result<String, JsValue> {
    let invalid = |kind: &str, message: &str, path: &DataPath<'_>| {
        diagnostic::error(category, kind, message, Some(&path.pointer()))
    };
    if !value.is_string() {
        return Err(invalid("InvalidInput", "expected a string", path));
    }
    let text: &JsString = value.unchecked_ref();
    if text.length() as usize > limit {
        return Err(invalid(
            "LimitExceeded",
            "string exceeds the admitted byte budget",
            path,
        ));
    }
    if !text.is_valid_utf16() {
        return Err(invalid(
            "InvalidUnicode",
            "unpaired UTF-16 surrogate is not DAML text",
            path,
        ));
    }
    let out = value.as_string().expect("admitted JavaScript string");
    if out.len() > limit {
        return Err(invalid(
            "LimitExceeded",
            "string exceeds the admitted byte budget",
            path,
        ));
    }
    Ok(out)
}

struct Fields {
    value: JsValue,
    keys: Vec<String>,
}

impl Fields {
    fn new(value: &JsValue, path: &DataPath<'_>) -> Result<Self, JsValue> {
        if !value.is_object() || value.is_null() || Array::is_array(value) {
            return Err(invalid("InvalidInput", "expected a semantic record", path));
        }
        let keys = Reflect::own_keys(value)?;
        if keys.length() > 5 {
            return Err(invalid(
                "InvalidInput",
                "unknown semantic record fields",
                path,
            ));
        }
        let keys = keys
            .iter()
            .map(|key| string(&key, 32, path))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            value: value.clone(),
            keys,
        })
    }

    fn allow(&self, allowed: &[&str], path: &DataPath<'_>) -> Result<(), JsValue> {
        if self.keys.iter().any(|key| !allowed.contains(&key.as_str())) {
            return Err(invalid(
                "InvalidInput",
                "unknown semantic record field",
                path,
            ));
        }
        Ok(())
    }

    fn get(&self, name: &str, path: &DataPath<'_>) -> Result<JsValue, JsValue> {
        self.optional(name)?.ok_or_else(|| {
            invalid(
                "InvalidInput",
                &format!("missing semantic field {name}"),
                path,
            )
        })
    }

    fn optional(&self, name: &str) -> Result<Option<JsValue>, JsValue> {
        if self.keys.iter().any(|key| key == name) {
            Ok(Some(Reflect::get(&self.value, &JsValue::from_str(name))?))
        } else {
            Ok(None)
        }
    }
}

#[derive(Default)]
struct Budget {
    bytes: usize,
    nodes: usize,
}

impl Budget {
    fn bytes(&mut self, count: usize, path: &DataPath<'_>) -> Result<(), JsValue> {
        if count > MAX_BYTES - self.bytes {
            return Err(invalid(
                "LimitExceeded",
                "semantic value exceeds the SDK byte budget",
                path,
            ));
        }
        self.bytes += count;
        Ok(())
    }

    fn node(&mut self, depth: usize, path: &DataPath<'_>) -> Result<(), JsValue> {
        if depth > MAX_DEPTH || self.nodes == MAX_NODES {
            return Err(invalid(
                "LimitExceeded",
                "semantic value exceeds the SDK node/depth budget",
                path,
            ));
        }
        self.nodes += 1;
        Ok(())
    }
}

fn array(value: JsValue, path: &DataPath<'_>) -> Result<Array, JsValue> {
    if !Array::is_array(&value) {
        return Err(invalid("InvalidInput", "expected an array", path));
    }
    let out: Array = value.unchecked_into();
    if out.length() as usize > MAX_NODES {
        return Err(invalid(
            "LimitExceeded",
            "semantic array exceeds the SDK node budget",
            path,
        ));
    }
    Ok(out)
}

pub(crate) fn decode(value: &JsValue) -> Result<SemanticNode, JsValue> {
    fn visit(
        value: &JsValue,
        path: &DataPath<'_>,
        depth: usize,
        budget: &mut Budget,
        ancestors: &WeakSet,
    ) -> Result<SemanticNode, JsValue> {
        budget.node(depth, path)?;
        let fields = Fields::new(value, path)?;
        let object: &Object = value.unchecked_ref();
        if ancestors.has(object) {
            return Err(invalid(
                "InvalidInput",
                "cyclic JavaScript semantic graph",
                path,
            ));
        }
        ancestors.add(object);
        let result = (|| {
            let kind = string(&fields.get("kind", path)?, 16, path)?;
            let body = match kind.as_str() {
                "null" => {
                    fields.allow(&["kind", "tag"], path)?;
                    SemanticValue::Null
                }
                "bool" => {
                    fields.allow(&["kind", "value", "tag"], path)?;
                    SemanticValue::Bool(
                        fields
                            .get("value", path)?
                            .as_bool()
                            .ok_or_else(|| invalid("InvalidInput", "expected a boolean", path))?,
                    )
                }
                "number" => {
                    fields.allow(&["kind", "value", "tag"], path)?;
                    let text = string(&fields.get("value", path)?, MAX_SCALAR, path)?;
                    budget.bytes(text.len(), path)?;
                    let number = text.parse().map_err(|_| {
                        invalid(
                            "InvalidNumber",
                            "number variant requires an exact JSON number lexeme",
                            path,
                        )
                    })?;
                    SemanticValue::Number(number)
                }
                "string" => {
                    fields.allow(&["kind", "value", "tag"], path)?;
                    let text = string(&fields.get("value", path)?, MAX_SCALAR, path)?;
                    budget.bytes(text.len(), path)?;
                    SemanticValue::String(text)
                }
                "list" => {
                    fields.allow(&["kind", "items", "tag"], path)?;
                    let values = array(fields.get("items", path)?, path)?;
                    let mut out = Vec::new();
                    for (index, value) in values.iter().enumerate() {
                        out.push(visit(
                            &value,
                            &path.index(index as u32),
                            depth + 1,
                            budget,
                            ancestors,
                        )?);
                    }
                    SemanticValue::Sequence(out)
                }
                "map" => {
                    fields.allow(&["kind", "entries", "tag"], path)?;
                    let entries = array(fields.get("entries", path)?, path)?;
                    let mut out = BTreeMap::new();
                    for entry in entries.iter() {
                        let entry = Fields::new(&entry, path)?;
                        entry.allow(&["key", "value"], path)?;
                        let key = string(&entry.get("key", path)?, MAX_SCALAR, path)?;
                        budget.bytes(key.len(), path)?;
                        let child = path.key(&key);
                        if out.contains_key(&key) {
                            return Err(invalid(
                                "InvalidInput",
                                "duplicate semantic mapping key",
                                &child,
                            ));
                        }
                        let value = visit(
                            &entry.get("value", &child)?,
                            &child,
                            depth + 1,
                            budget,
                            ancestors,
                        )?;
                        out.insert(key, value);
                    }
                    SemanticValue::Mapping(out)
                }
                _ => return Err(invalid("InvalidInput", "unknown semantic value kind", path)),
            };
            let tag = fields
                .optional("tag")?
                .map(|tag| {
                    let tag = Fields::new(&tag, path)?;
                    tag.allow(&["name", "binding"], path)?;
                    let name = string(&tag.get("name", path)?, MAX_SCALAR, path)?;
                    budget.bytes(name.len(), path)?;
                    let binding = tag
                        .optional("binding")?
                        .map(|value| {
                            let out = string(&value, MAX_SCALAR, path)?;
                            budget.bytes(out.len(), path)?;
                            Ok::<_, JsValue>(out)
                        })
                        .transpose()?;
                    Ok::<_, JsValue>(SemanticTag { name, binding })
                })
                .transpose()?;
            Ok(SemanticNode { tag, value: body })
        })();
        ancestors.delete(object);
        result
    }
    visit(
        value,
        &DataPath::root(),
        0,
        &mut Budget::default(),
        &WeakSet::new(),
    )
}

pub(crate) fn sources(value: &JsValue) -> Result<Vec<(String, String)>, JsValue> {
    if !value.is_object() || value.is_null() || Array::is_array(value) {
        return Err(diagnostic::error(
            "native",
            "InvalidInput",
            "expected a definition-resource record",
            Some(""),
        ));
    }
    let keys = Reflect::own_keys(value)?;
    if keys.length() > 10_000 {
        return Err(diagnostic::error(
            "native",
            "LimitExceeded",
            "too many definition resources",
            Some(""),
        ));
    }
    let mut out = Vec::with_capacity(keys.length() as usize);
    let mut bytes = 0;
    for key in keys.iter() {
        let key_text = resource_string(&key, 4096, &DataPath::root())?;
        let path = DataPath::root();
        let path = path.key(&key_text);
        let text = resource_string(&Reflect::get(value, &key)?, MAX_BYTES - bytes, &path)?;
        bytes += text.len();
        out.push((key_text, text));
    }
    Ok(out)
}

fn node(
    kind: &str,
    tag: Option<(&str, Option<&str>)>,
    budget: &mut Budget,
    path: &DataPath<'_>,
) -> Result<Object, JsValue> {
    let out = Object::new();
    set(&out, "kind", &JsValue::from_str(kind))?;
    if let Some((name, binding)) = tag {
        budget.bytes(name.len() + binding.map_or(0, str::len), path)?;
        let value = Object::new();
        set(&value, "name", &JsValue::from_str(name))?;
        if let Some(binding) = binding {
            set(&value, "binding", &JsValue::from_str(binding))?;
        }
        set(&out, "tag", &value)?;
    }
    Ok(out)
}

pub(crate) fn document(document: &Document) -> Result<JsValue, JsValue> {
    fn visit(
        document: &Document,
        value: &Node,
        path: &DataPath<'_>,
        depth: usize,
        budget: &mut Budget,
    ) -> Result<JsValue, JsValue> {
        budget.node(depth, path)?;
        let tag = document.tag(value).map(|name| (name, None));
        let (kind, scalar) = match &value.value {
            Value::Null => ("null", None),
            Value::Bool(value) => ("bool", Some(JsValue::from_bool(*value))),
            Value::Number(span) => {
                let value = document.slice(*span).expect("parsed number span");
                budget.bytes(value.len(), path)?;
                ("number", Some(JsValue::from_str(value)))
            }
            Value::String(_) => {
                let value = document.string(value).expect("parsed string");
                budget.bytes(value.len(), path)?;
                ("string", Some(JsValue::from_str(value)))
            }
            Value::Sequence(_) => ("list", None),
            Value::Mapping(_) => ("map", None),
        };
        let out = node(kind, tag, budget, path)?;
        if let Some(scalar) = scalar {
            set(&out, "value", &scalar)?;
        }
        match &value.value {
            Value::Sequence(values) => {
                let items = Array::new();
                for (index, value) in values.iter().enumerate() {
                    items.push(&visit(
                        document,
                        value,
                        &path.index(index as u32),
                        depth + 1,
                        budget,
                    )?);
                }
                set(&out, "items", &items)?;
            }
            Value::Mapping(values) => {
                let entries = Array::new();
                for value in values {
                    let key = document.slice(value.key).expect("parsed mapping key");
                    budget.bytes(key.len(), path)?;
                    let entry = Object::new();
                    set(&entry, "key", &JsValue::from_str(key))?;
                    set(
                        &entry,
                        "value",
                        &visit(document, &value.value, &path.key(key), depth + 1, budget)?,
                    )?;
                    entries.push(&entry);
                }
                set(&out, "entries", &entries)?;
            }
            _ => (),
        }
        Ok(out.into())
    }
    visit(
        document,
        document.root(),
        &DataPath::root(),
        0,
        &mut Budget::default(),
    )
}

pub(crate) fn resolution(root: &ResolvedNode<'_>) -> Result<(JsValue, JsValue), JsValue> {
    fn visit(
        value: &ResolvedNode<'_>,
        path: &DataPath<'_>,
        depth: usize,
        budget: &mut Budget,
        metadata: &mut Budget,
        origins: &Object,
    ) -> Result<JsValue, JsValue> {
        budget.node(depth, path)?;
        let source_bytes = match value.origin() {
            Origin::Input(_) => 0,
            Origin::Default(location) => location.resource.len() + location.pointer.len(),
        };
        let tag_bytes = match value.tag_origin() {
            Some(Origin::Default(location)) => location.resource.len() + location.pointer.len(),
            _ => 0,
        };
        metadata.bytes(path.len + source_bytes + tag_bytes, path)?;
        let origin = diagnostic::origin(value.origin())?;
        if let Some(tag_origin) = value.tag_origin() {
            set(&origin, "tagOrigin", &diagnostic::origin(tag_origin)?)?;
        }
        set(origins, &path.pointer(), &origin)?;
        let tag = value.tag().map(|name| (name, value.tag_binding()));
        let (kind, scalar) = match value.value() {
            ResolvedValue::Null => ("null", None),
            ResolvedValue::Bool(value) => ("bool", Some(JsValue::from_bool(*value))),
            ResolvedValue::Number(value) => {
                let value = value.as_ref().as_str();
                budget.bytes(value.len(), path)?;
                ("number", Some(JsValue::from_str(value)))
            }
            ResolvedValue::String(value) => {
                budget.bytes(value.len(), path)?;
                ("string", Some(JsValue::from_str(value)))
            }
            ResolvedValue::Sequence(_) => ("list", None),
            ResolvedValue::Mapping(_) => ("map", None),
        };
        let out = node(kind, tag, budget, path)?;
        if let Some(scalar) = scalar {
            set(&out, "value", &scalar)?;
        }
        match value.value() {
            ResolvedValue::Sequence(values) => {
                let items = Array::new();
                for (index, value) in values.iter().enumerate() {
                    items.push(&visit(
                        value,
                        &path.index(index as u32),
                        depth + 1,
                        budget,
                        metadata,
                        origins,
                    )?);
                }
                set(&out, "items", &items)?;
            }
            ResolvedValue::Mapping(values) => {
                let entries = Array::new();
                for (key, value) in values {
                    budget.bytes(key.len(), path)?;
                    let entry = Object::new();
                    set(&entry, "key", &JsValue::from_str(key))?;
                    set(
                        &entry,
                        "value",
                        &visit(value, &path.key(key), depth + 1, budget, metadata, origins)?,
                    )?;
                    entries.push(&entry);
                }
                set(&out, "entries", &entries)?;
            }
            _ => (),
        }
        Ok(out.into())
    }
    let origins = Object::new();
    let value = visit(
        root,
        &DataPath::root(),
        0,
        &mut Budget::default(),
        &mut Budget::default(),
        &origins,
    )?;
    Ok((value, origins.into()))
}
