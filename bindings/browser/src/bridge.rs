use std::collections::BTreeMap;

use daml::{
    AdmissionStage, CopyEdge, Document, Node, NodeSummary, Origin, ResolvedNode, ResolvedValue,
    SemanticNode, SemanticTag, SemanticValue, StageEdit, StageOutcome, SummaryValue, TemporalKind,
    TemporalValue, Value,
};
use js_sys::{Array, Function, JsString, Object, Reflect, Uint8Array, WeakSet};
use wasm_bindgen::{JsCast, prelude::*};

use crate::diagnostic::{self, set};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = Array, js_name = isArray, catch)]
    fn is_array(value: &JsValue) -> Result<bool, JsValue>;
}

#[wasm_bindgen(module = "/src/byte-view.js")]
extern "C" {
    #[wasm_bindgen(js_name = isStageRejection, catch)]
    fn is_stage_rejection(value: &JsValue) -> Result<bool, JsValue>;

    #[wasm_bindgen(js_name = isMeaningBytes, catch)]
    fn is_meaning_bytes(value: &JsValue) -> Result<bool, JsValue>;

    #[wasm_bindgen(js_name = meaningLength, catch)]
    fn meaning_length(value: &JsValue) -> Result<f64, JsValue>;

    #[wasm_bindgen(js_name = copyMeaning, catch)]
    fn copy_meaning(value: &JsValue, destination: &mut [u8]) -> Result<(), JsValue>;
}

pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_SCALAR: usize = 1024 * 1024;
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 128;

#[derive(Clone, Copy)]
enum Segment<'a> {
    Root,
    Key(&'a str),
    Index(u32),
    Pointer(&'a str),
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
                Segment::Pointer(value) => out.push_str(value),
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

struct Fields<'a> {
    value: &'a JsValue,
    keys: Vec<String>,
}

impl<'a> Fields<'a> {
    fn new(value: &'a JsValue, path: &DataPath<'_>) -> Result<Self, JsValue> {
        if !value.is_object() || value.is_null() || is_array(value)? {
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
        Ok(Self { value, keys })
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
            Ok(Some(Reflect::get(self.value, &JsValue::from_str(name))?))
        } else {
            Ok(None)
        }
    }
}

#[derive(Default)]
pub(crate) struct Budget {
    bytes: usize,
    nodes: usize,
}

impl Budget {
    pub(crate) fn text(&mut self, value: &JsValue, path: &DataPath<'_>) -> Result<String, JsValue> {
        let out = string(value, MAX_SCALAR.min(MAX_BYTES - self.bytes), path)?;
        self.bytes(out.len(), path)?;
        Ok(out)
    }

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

fn array(value: JsValue, path: &DataPath<'_>) -> Result<(JsValue, u32), JsValue> {
    if !is_array(&value)? {
        return Err(invalid("InvalidInput", "expected an array", path));
    }
    let length = Reflect::get(&value, &JsValue::from_str("length"))?
        .as_f64()
        .filter(|length| length.is_finite() && length.fract() == 0.0 && *length >= 0.0)
        .ok_or_else(|| invalid("InvalidInput", "expected an array length", path))?;
    if length > MAX_NODES as f64 {
        return Err(invalid(
            "LimitExceeded",
            "semantic array exceeds the SDK node budget",
            path,
        ));
    }
    Ok((value, length as u32))
}

pub(crate) fn decode(value: &JsValue) -> Result<SemanticNode, JsValue> {
    decode_with_budget(value, &mut Budget::default())
}

pub(crate) fn decode_with_budget(
    value: &JsValue,
    budget: &mut Budget,
) -> Result<SemanticNode, JsValue> {
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
                    let text = budget.text(&fields.get("value", path)?, path)?;
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
                    let text = budget.text(&fields.get("value", path)?, path)?;
                    SemanticValue::String(text)
                }
                "date" | "time" | "datetime" => {
                    fields.allow(&["kind", "value", "tag"], path)?;
                    let temporal_kind = match kind.as_str() {
                        "date" => TemporalKind::Date,
                        "time" => TemporalKind::Time,
                        _ => TemporalKind::DateTime,
                    };
                    let text = budget.text(&fields.get("value", path)?, path)?;
                    SemanticValue::Temporal(
                        TemporalValue::parse(temporal_kind, text)
                            .map_err(|error| invalid("InvalidInput", &error.to_string(), path))?,
                    )
                }
                "list" => {
                    fields.allow(&["kind", "items", "tag"], path)?;
                    let (values, length) = array(fields.get("items", path)?, path)?;
                    let mut out = Vec::new();
                    for index in 0..length {
                        let value = Reflect::get_u32(&values, index)?;
                        out.push(visit(
                            &value,
                            &path.index(index),
                            depth + 1,
                            budget,
                            ancestors,
                        )?);
                    }
                    SemanticValue::Sequence(out)
                }
                "map" => {
                    fields.allow(&["kind", "entries", "tag"], path)?;
                    let (entries, length) = array(fields.get("entries", path)?, path)?;
                    let mut out = BTreeMap::new();
                    for index in 0..length {
                        let entry = Reflect::get_u32(&entries, index)?;
                        let entry = Fields::new(&entry, path)?;
                        entry.allow(&["key", "value"], path)?;
                        let key = budget.text(&entry.get("key", path)?, path)?;
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
                    let name = budget.text(&tag.get("name", path)?, path)?;
                    let binding = tag
                        .optional("binding")?
                        .map(|value| budget.text(&value, path))
                        .transpose()?;
                    Ok::<_, JsValue>(SemanticTag { name, binding })
                })
                .transpose()?;
            Ok(SemanticNode { tag, value: body })
        })();
        ancestors.delete(object);
        result
    }
    visit(value, &DataPath::root(), 0, budget, &WeakSet::new())
}

pub(crate) fn sources(value: &JsValue) -> Result<Vec<(String, String)>, JsValue> {
    if !value.is_object() || value.is_null() || is_array(value)? {
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
            Value::Temporal(_) => {
                let value = document.temporal(value).expect("parsed temporal");
                budget.bytes(value.as_str().len(), path)?;
                (
                    value.kind().as_str(),
                    Some(JsValue::from_str(value.as_str())),
                )
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

enum SnapshotOrigins<'a> {
    Separate { output: &'a Object, budget: Budget },
    Shared(&'a Object),
}

pub(crate) fn resolution(root: &ResolvedNode<'_>) -> Result<(JsValue, JsValue), JsValue> {
    let origins = Object::new();
    let value = resolved_snapshot(
        root,
        &mut Budget::default(),
        Some(SnapshotOrigins::Separate {
            output: &origins,
            budget: Budget::default(),
        }),
    )?;
    Ok((value, origins.into()))
}

pub(crate) fn resolution_with_budget(
    root: &ResolvedNode<'_>,
    budget: &mut Budget,
) -> Result<(JsValue, JsValue), JsValue> {
    let origins = Object::new();
    let value = resolved_snapshot(root, budget, Some(SnapshotOrigins::Shared(&origins)))?;
    Ok((value, origins.into()))
}

fn resolved_snapshot(
    root: &ResolvedNode<'_>,
    budget: &mut Budget,
    mut origins: Option<SnapshotOrigins<'_>>,
) -> Result<JsValue, JsValue> {
    fn visit(
        value: &ResolvedNode<'_>,
        path: &DataPath<'_>,
        depth: usize,
        budget: &mut Budget,
        origins: &mut Option<SnapshotOrigins<'_>>,
    ) -> Result<JsValue, JsValue> {
        budget.node(depth, path)?;
        if let Some(origins) = origins {
            let source_bytes = match value.origin() {
                Origin::Input(_) | Origin::Semantic => 0,
                Origin::Default(location) => location.resource.len() + location.pointer.len(),
            };
            let tag_bytes = match value.tag_origin() {
                Some(Origin::Default(location)) => location.resource.len() + location.pointer.len(),
                _ => 0,
            };
            let (origins, provenance_budget) = match origins {
                SnapshotOrigins::Separate { output, budget } => (*output, budget),
                SnapshotOrigins::Shared(output) => (*output, &mut *budget),
            };
            provenance_budget.bytes(path.len + source_bytes + tag_bytes, path)?;
            let origin = diagnostic::origin(value.origin())?;
            if let Some(tag_origin) = value.tag_origin() {
                set(&origin, "tagOrigin", &diagnostic::origin(tag_origin)?)?;
            }
            set(origins, &path.pointer(), &origin)?;
        }
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
            ResolvedValue::Temporal(value) => {
                budget.bytes(value.as_str().len(), path)?;
                (
                    value.kind().as_str(),
                    Some(JsValue::from_str(value.as_str())),
                )
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
                        &visit(value, &path.key(key), depth + 1, budget, origins)?,
                    )?;
                    entries.push(&entry);
                }
                set(&out, "entries", &entries)?;
            }
            _ => (),
        }
        Ok(out.into())
    }
    visit(root, &DataPath::root(), 0, budget, &mut origins)
}

pub(crate) fn additional_types(value: Option<&JsValue>) -> Result<Vec<String>, JsValue> {
    let mut budget = Budget::default();
    let path = DataPath::root();
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let (values, length) = array(value.clone(), &path)?;
    let mut out = Vec::new();
    for index in 0..length {
        let value = Reflect::get_u32(&values, index)?;
        budget.node(0, &path)?;
        out.push(budget.text(&value, &path)?);
    }
    Ok(out)
}

fn summary_value(
    value: SummaryValue<'_>,
    budget: &mut Budget,
    path: &DataPath<'_>,
) -> Result<Object, JsValue> {
    budget.node(0, path)?;
    let (kind, scalar, length) = match value {
        SummaryValue::Null => ("null", None, None),
        SummaryValue::Bool(value) => ("bool", Some(JsValue::from_bool(value)), None),
        SummaryValue::Number(value) => {
            budget.bytes(value.len(), path)?;
            ("number", Some(JsValue::from_str(value)), None)
        }
        SummaryValue::String(value) => {
            budget.bytes(value.len(), path)?;
            ("string", Some(JsValue::from_str(value)), None)
        }
        SummaryValue::Temporal { kind, value } => {
            budget.bytes(value.len(), path)?;
            (kind.as_str(), Some(JsValue::from_str(value)), None)
        }
        SummaryValue::Sequence(length) => ("list", None, Some(length)),
        SummaryValue::Mapping(length) => ("map", None, Some(length)),
    };
    let out = node(kind, None, budget, path)?;
    if let Some(scalar) = scalar {
        set(&out, "value", &scalar)?;
    }
    if let Some(length) = length {
        set(&out, "length", &JsValue::from_f64(length as f64))?;
    }
    Ok(out)
}

/// Invocation-local synchronous host adapter. All SDK-owned bridge work shares one budget.
pub(crate) struct Stage {
    host: JsValue,
    defer: Function,
    materialize: Function,
    number_class: Option<Function>,
    pub(crate) budget: Budget,
}

impl Stage {
    pub(crate) fn new(host: JsValue) -> Result<Self, JsValue> {
        let path = DataPath::root();
        if !host.is_object() || host.is_null() {
            return Err(invalid(
                "InvalidInput",
                "expected an admission stage object",
                &path,
            ));
        }
        let method = |name: &str, optional: bool| -> Result<Option<Function>, JsValue> {
            let value = Reflect::get(&host, &JsValue::from_str(name))?;
            if optional && value.is_undefined() {
                return Ok(None);
            }
            value
                .dyn_into::<Function>()
                .map(Some)
                .map_err(|_| invalid("InvalidInput", "stage callback must be a function", &path))
        };
        let defer = method("defer", false)?.expect("required callback");
        let materialize = method("materialize", false)?.expect("required callback");
        let number_class = method("numberClass", true)?;
        Ok(Self {
            host,
            defer,
            materialize,
            number_class,
            budget: Budget::default(),
        })
    }

    pub(crate) fn edits(&mut self, value: &JsValue) -> Result<Vec<StageEdit>, JsValue> {
        let path = DataPath::root();
        let (values, length) = array(value.clone(), &path)?;
        let mut edits = Vec::new();
        for index in 0..length {
            let value = Reflect::get_u32(&values, index)?;
            let path = path.index(index);
            self.budget.node(0, &path)?;
            let fields = Fields::new(&value, &path)?;
            let op = string(&fields.get("op", &path)?, 16, &path)?;
            let destination = self.budget.text(&fields.get("path", &path)?, &path)?;
            let edit = match op.as_str() {
                "replace" => {
                    fields.allow(&["op", "path", "value"], &path)?;
                    StageEdit::Replace {
                        path: destination,
                        value: decode_with_budget(&fields.get("value", &path)?, &mut self.budget)?,
                    }
                }
                "copy" => {
                    fields.allow(&["op", "path", "from"], &path)?;
                    StageEdit::Copy {
                        path: destination,
                        from: self.budget.text(&fields.get("from", &path)?, &path)?,
                    }
                }
                "expect" => {
                    fields.allow(&["op", "path", "name"], &path)?;
                    StageEdit::Expect {
                        path: destination,
                        name: self.budget.text(&fields.get("name", &path)?, &path)?,
                    }
                }
                _ => {
                    return Err(invalid(
                        "InvalidInput",
                        "unknown stage edit operation",
                        &path,
                    ));
                }
            };
            edits.push(edit);
        }
        Ok(edits)
    }
}

impl AdmissionStage for Stage {
    type Error = JsValue;

    fn defer(&mut self, summary: NodeSummary<'_>, path: &str) -> Result<bool, JsValue> {
        let path = DataPath {
            parent: None,
            segment: Segment::Pointer(path),
            len: path.len(),
        };
        self.budget.bytes(path.len, &path)?;
        let out = summary_value(summary.value(), &mut self.budget, &path)?;
        if let Some(name) = summary.tag() {
            let binding = summary.tag_binding();
            self.budget
                .bytes(name.len() + binding.map_or(0, str::len), &path)?;
            let tag = Object::new();
            set(&tag, "name", &JsValue::from_str(name))?;
            if let Some(binding) = binding {
                set(&tag, "binding", &JsValue::from_str(binding))?;
            }
            set(&out, "tag", &tag)?;
        }
        if matches!(summary.value(), SummaryValue::Mapping(_)) {
            let fields = Array::new();
            for (key, value) in summary.fields() {
                self.budget.bytes(key.len(), &path)?;
                let entry = Object::new();
                set(&entry, "key", &JsValue::from_str(key))?;
                set(
                    &entry,
                    "summary",
                    summary_value(value, &mut self.budget, &path)?.as_ref(),
                )?;
                fields.push(&entry);
            }
            set(&out, "fields", &fields)?;
        }
        self.defer
            .call2(&self.host, &out, &JsValue::from_str(&path.pointer()))?
            .as_bool()
            .ok_or_else(|| invalid("InvalidInput", "stage defer must return a boolean", &path))
    }

    fn materialize(
        &mut self,
        root: &ResolvedNode<'_>,
        copies: &[CopyEdge],
        round: usize,
    ) -> Result<StageOutcome<JsValue>, JsValue> {
        let path = DataPath::root();
        let value = resolved_snapshot(root, &mut self.budget, None)?;
        let history = Array::new();
        for copy in copies {
            self.budget.node(0, &path)?;
            self.budget
                .bytes(copy.path.len() + copy.from.len(), &path)?;
            let edge = Object::new();
            set(&edge, "path", &JsValue::from_str(&copy.path))?;
            set(&edge, "from", &JsValue::from_str(&copy.from))?;
            history.push(&edge);
        }
        let result = self.materialize.call3(
            &self.host,
            &value,
            &history,
            &JsValue::from_f64(round as f64),
        )?;
        if result.is_null() || result.is_undefined() {
            return Ok(StageOutcome::Ready { meaning: None });
        }
        // Admit external arrays before instanceof can traverse a proxy's prototypes.
        if is_array(&result)? {
            return self.edits(&result).map(StageOutcome::Edits);
        }
        if is_stage_rejection(&result)? {
            return Ok(StageOutcome::Reject(result));
        }
        if is_meaning_bytes(&result)? {
            let length = meaning_length(&result)? as usize;
            self.budget.bytes(length, &path)?;
            // The destination is initialized and its JS view is sized by Rust, never
            // by an ordinary property read on the caller's byte view.
            let mut bytes = vec![0; length];
            copy_meaning(&result, &mut bytes)?;
            return Ok(StageOutcome::Ready {
                meaning: Some(bytes),
            });
        }
        Err(invalid(
            "InvalidInput",
            "stage materialize must return null, undefined, Uint8Array, edits, or Error",
            &path,
        ))
    }

    fn number_class(&mut self, lexeme: &str) -> Result<u32, JsValue> {
        let Some(callback) = &self.number_class else {
            return Ok(0);
        };
        let path = DataPath::root();
        self.budget.node(0, &path)?;
        self.budget.bytes(lexeme.len(), &path)?;
        let value = callback.call1(&self.host, &JsValue::from_str(lexeme))?;
        let value = value
            .as_f64()
            .filter(|value| {
                value.is_finite() && value.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(value)
            })
            .ok_or_else(|| {
                invalid(
                    "InvalidInput",
                    "numberClass must return a u32 integer",
                    &path,
                )
            })?;
        Ok(value as u32)
    }
}

pub(crate) fn meaning(value: Option<&[u8]>, budget: &mut Budget) -> Result<JsValue, JsValue> {
    match value {
        None => Ok(JsValue::NULL),
        Some(value) => {
            budget.bytes(value.len(), &DataPath::root())?;
            Ok(Uint8Array::from(value).into())
        }
    }
}
