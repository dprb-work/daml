use std::{collections::BTreeMap, fmt::Write as _};

use daml_core::{Document, Node, SemanticNode, SemanticTag, SemanticValue, Value};
use pyo3::{
    prelude::*,
    types::{PyBool, PyDict, PyList, PyString},
};

use crate::{SemanticError, number_to_python, push_pointer_key};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_SCALAR: usize = 1024 * 1024;
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 128;

pub(crate) fn invalid(py: Python<'_>, kind: &str, message: &str, pointer: &str) -> PyErr {
    let error = SemanticError::new_err(format!("{message} at {pointer:?}"));
    if let Err(error) = error.value(py).setattr("kind", kind) {
        return error;
    }
    if let Err(error) = error.value(py).setattr("pointer", pointer) {
        return error;
    }
    error
}

pub(crate) fn fields<'a, 'py>(
    value: &'a Bound<'py, PyAny>,
    allowed: &[&str],
    pointer: &str,
) -> PyResult<&'a Bound<'py, PyDict>> {
    let py = value.py();
    let fields = value
        .cast::<PyDict>()
        .map_err(|_| invalid(py, "InvalidInput", "expected a semantic record", pointer))?;
    if fields.len() > allowed.len() {
        return Err(invalid(
            py,
            "InvalidInput",
            "unknown semantic record field",
            pointer,
        ));
    }
    for (key, _) in fields.iter() {
        let key = text(&key, 32, pointer)?;
        if !allowed.contains(&key) {
            return Err(invalid(
                py,
                "InvalidInput",
                "unknown semantic record field",
                pointer,
            ));
        }
    }
    Ok(fields)
}

pub(crate) fn required<'py>(
    fields: &Bound<'py, PyDict>,
    name: &str,
    pointer: &str,
) -> PyResult<Bound<'py, PyAny>> {
    fields.get_item(name)?.ok_or_else(|| {
        invalid(
            fields.py(),
            "InvalidInput",
            &format!("missing semantic field {name}"),
            pointer,
        )
    })
}

pub(crate) fn text<'a>(
    value: &'a Bound<'_, PyAny>,
    limit: usize,
    pointer: &str,
) -> PyResult<&'a str> {
    let py = value.py();
    let string = value
        .cast::<PyString>()
        .map_err(|_| invalid(py, "InvalidInput", "expected a string", pointer))?;
    // Bound code points before asking Python to materialize a UTF-8 representation.
    if string.code_point_len()? > limit {
        return Err(invalid(
            py,
            "LimitExceeded",
            "string exceeds scalar byte bound",
            pointer,
        ));
    }
    let string = string.to_str().map_err(|cause| {
        let error = invalid(
            py,
            "InvalidInput",
            "string contains invalid Unicode",
            pointer,
        );
        error.set_cause(py, Some(cause));
        error
    })?;
    if string.len() > limit {
        return Err(invalid(
            py,
            "LimitExceeded",
            "string exceeds scalar byte bound",
            pointer,
        ));
    }
    Ok(string)
}

#[derive(Default)]
pub(crate) struct Budget {
    bytes: usize,
    nodes: usize,
}

impl Budget {
    pub(crate) fn charge(
        &mut self,
        py: Python<'_>,
        nodes: usize,
        bytes: usize,
        pointer: &str,
    ) -> PyResult<()> {
        if nodes > MAX_NODES - self.nodes || bytes > MAX_BYTES - self.bytes {
            return Err(invalid(
                py,
                "LimitExceeded",
                "semantic work exceeds cumulative node/byte budget",
                pointer,
            ));
        }
        self.nodes += nodes;
        self.bytes += bytes;
        Ok(())
    }

    pub(crate) fn borrow_string<'a>(
        &mut self,
        value: &'a Bound<'_, PyAny>,
        pointer: &str,
    ) -> PyResult<&'a str> {
        let text = text(value, MAX_SCALAR.min(MAX_BYTES - self.bytes), pointer)?;
        self.bytes += text.len();
        Ok(text)
    }

    pub(crate) fn string(&mut self, value: &Bound<'_, PyAny>, pointer: &str) -> PyResult<String> {
        Ok(self.borrow_string(value, pointer)?.to_owned())
    }

    fn array<'a, 'py>(
        &self,
        value: &'a Bound<'py, PyAny>,
        pointer: &str,
    ) -> PyResult<&'a Bound<'py, PyList>> {
        let list = value
            .cast::<PyList>()
            .map_err(|_| invalid(value.py(), "InvalidInput", "expected a list", pointer))?;
        if list.len() > MAX_NODES - self.nodes {
            return Err(invalid(
                value.py(),
                "LimitExceeded",
                "semantic array exceeds node budget",
                pointer,
            ));
        }
        Ok(list)
    }
}

pub(crate) fn decode(value: &Bound<'_, PyAny>) -> PyResult<SemanticNode> {
    decode_with_budget(value, &mut Budget::default())
}

pub(crate) fn decode_with_budget(
    value: &Bound<'_, PyAny>,
    budget: &mut Budget,
) -> PyResult<SemanticNode> {
    fn visit(
        value: &Bound<'_, PyAny>,
        pointer: &mut String,
        depth: usize,
        budget: &mut Budget,
        ancestors: &mut Vec<usize>,
    ) -> PyResult<SemanticNode> {
        let py = value.py();
        if depth > MAX_DEPTH || budget.nodes == MAX_NODES {
            return Err(invalid(
                py,
                "LimitExceeded",
                "semantic value exceeds node/depth budget",
                pointer,
            ));
        }
        budget.nodes += 1;
        let identity = value.as_ptr() as usize;
        if ancestors.contains(&identity) {
            return Err(invalid(
                py,
                "InvalidInput",
                "cyclic Python semantic graph",
                pointer,
            ));
        }
        let record = value
            .cast::<PyDict>()
            .map_err(|_| invalid(py, "InvalidInput", "expected a semantic record", pointer))?;
        if record.len() > 3 {
            return Err(invalid(
                py,
                "InvalidInput",
                "unknown semantic record field",
                pointer,
            ));
        }
        ancestors.push(identity);
        let result = (|| {
            let kind = required(record, "kind", pointer)?;
            let kind = text(&kind, 16, pointer)?;
            let allowed: &[&str] = match kind {
                "null" => &["kind", "tag"],
                "bool" | "number" | "string" => &["kind", "value", "tag"],
                "list" => &["kind", "items", "tag"],
                "map" => &["kind", "entries", "tag"],
                _ => {
                    return Err(invalid(
                        py,
                        "InvalidInput",
                        "unknown semantic value kind",
                        pointer,
                    ));
                }
            };
            fields(value, allowed, pointer)?;
            if depth >= MAX_DEPTH && matches!(kind, "list" | "map") {
                return Err(invalid(
                    py,
                    "LimitExceeded",
                    "semantic value exceeds depth budget",
                    pointer,
                ));
            }
            let tag = record
                .get_item("tag")?
                .map(|tag| {
                    let record = fields(&tag, &["name", "binding"], pointer)?;
                    let name = budget.string(&required(record, "name", pointer)?, pointer)?;
                    let binding = record
                        .get_item("binding")?
                        .map(|binding| budget.string(&binding, pointer))
                        .transpose()?;
                    Ok::<_, PyErr>(SemanticTag { name, binding })
                })
                .transpose()?;
            let body = match kind {
                "null" => SemanticValue::Null,
                "bool" => {
                    let value = required(record, "value", pointer)?;
                    if !value.is_instance_of::<PyBool>() {
                        return Err(invalid(py, "InvalidInput", "expected a boolean", pointer));
                    }
                    SemanticValue::Bool(value.extract()?)
                }
                "number" => {
                    let value = required(record, "value", pointer)?;
                    // Charge the budget before Number's owned exact-lexeme allocation.
                    let raw = text(&value, MAX_SCALAR.min(MAX_BYTES - budget.bytes), pointer)?;
                    budget.bytes += raw.len();
                    SemanticValue::Number(raw.parse().map_err(|_| {
                        invalid(
                            py,
                            "InvalidNumber",
                            "number variant requires an exact JSON number lexeme",
                            pointer,
                        )
                    })?)
                }
                "string" => SemanticValue::String(
                    budget.string(&required(record, "value", pointer)?, pointer)?,
                ),
                "list" => {
                    let items = required(record, "items", pointer)?;
                    let items = budget.array(&items, pointer)?;
                    let mut out = Vec::with_capacity(items.len());
                    let parent = pointer.len();
                    for (index, item) in items.iter().enumerate() {
                        write!(pointer, "/{index}").expect("writing to a String cannot fail");
                        let child = visit(&item, pointer, depth + 1, budget, ancestors);
                        pointer.truncate(parent);
                        out.push(child?);
                    }
                    SemanticValue::Sequence(out)
                }
                "map" => {
                    let entries = required(record, "entries", pointer)?;
                    let entries = budget.array(&entries, pointer)?;
                    let mut out = BTreeMap::new();
                    let parent = pointer.len();
                    for entry in entries.iter() {
                        let entry = fields(&entry, &["key", "value"], pointer)?;
                        let key = budget.string(&required(entry, "key", pointer)?, pointer)?;
                        push_pointer_key(pointer, &key);
                        if out.contains_key(&key) {
                            return Err(invalid(
                                py,
                                "InvalidInput",
                                "duplicate semantic mapping key",
                                pointer,
                            ));
                        }
                        let child = visit(
                            &required(entry, "value", pointer)?,
                            pointer,
                            depth + 1,
                            budget,
                            ancestors,
                        );
                        pointer.truncate(parent);
                        out.insert(key, child?);
                    }
                    SemanticValue::Mapping(out)
                }
                _ => unreachable!("semantic kinds were admitted above"),
            };
            Ok(SemanticNode { tag, value: body })
        })();
        ancestors.pop();
        result
    }
    visit(value, &mut String::new(), 0, budget, &mut Vec::new())
}

pub(crate) fn document<'py>(
    py: Python<'py>,
    document: &Document,
    node: &Node,
) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    let kind = match &node.value {
        Value::Null => "null",
        Value::Bool(value) => {
            out.set_item("value", value)?;
            "bool"
        }
        Value::Number(span) => {
            out.set_item(
                "value",
                document.slice(*span).expect("parsed number has valid span"),
            )?;
            "number"
        }
        Value::String(_) => {
            out.set_item(
                "value",
                document.string(node).expect("parsed string has valid data"),
            )?;
            "string"
        }
        Value::Sequence(items) => {
            let out_items = PyList::empty(py);
            for item in items {
                out_items.append(self::document(py, document, item)?)?;
            }
            out.set_item("items", out_items)?;
            "list"
        }
        Value::Mapping(entries) => {
            let out_entries = PyList::empty(py);
            for entry in entries {
                let out_entry = PyDict::new(py);
                out_entry.set_item(
                    "key",
                    document
                        .slice(entry.key)
                        .expect("parsed key has valid span"),
                )?;
                out_entry.set_item("value", self::document(py, document, &entry.value)?)?;
                out_entries.append(out_entry)?;
            }
            out.set_item("entries", out_entries)?;
            "map"
        }
    };
    out.set_item("kind", kind)?;
    if let Some(name) = document.tag(node) {
        let tag = PyDict::new(py);
        tag.set_item("name", name)?;
        out.set_item("tag", tag)?;
    }
    Ok(out)
}

pub(crate) fn encode<'py>(py: Python<'py>, node: &SemanticNode) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    let kind = match &node.value {
        SemanticValue::Null => "null",
        SemanticValue::Bool(value) => {
            out.set_item("value", value)?;
            "bool"
        }
        SemanticValue::Number(value) => {
            out.set_item("value", value.as_str())?;
            "number"
        }
        SemanticValue::String(value) => {
            out.set_item("value", value)?;
            "string"
        }
        SemanticValue::Sequence(items) => {
            let out_items = PyList::empty(py);
            for item in items {
                out_items.append(encode(py, item)?)?;
            }
            out.set_item("items", out_items)?;
            "list"
        }
        SemanticValue::Mapping(entries) => {
            let out_entries = PyList::empty(py);
            for (key, value) in entries {
                let entry = PyDict::new(py);
                entry.set_item("key", key)?;
                entry.set_item("value", encode(py, value)?)?;
                out_entries.append(entry)?;
            }
            out.set_item("entries", out_entries)?;
            "map"
        }
    };
    out.set_item("kind", kind)?;
    if let Some(tag) = &node.tag {
        let out_tag = PyDict::new(py);
        out_tag.set_item("name", &tag.name)?;
        if let Some(binding) = &tag.binding {
            out_tag.set_item("binding", binding)?;
        }
        out.set_item("tag", out_tag)?;
    }
    Ok(out)
}

pub(crate) fn to_python<'py>(
    py: Python<'py>,
    node: &SemanticNode,
    decimal: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    match &node.value {
        SemanticValue::Null => Ok(py.None().into_bound(py)),
        SemanticValue::Bool(value) => Ok(PyBool::new(py, *value).to_owned().into_any()),
        SemanticValue::Number(value) => number_to_python(py, value.as_str(), decimal),
        SemanticValue::String(value) => Ok(PyString::new(py, value).into_any()),
        SemanticValue::Sequence(items) => {
            let out = PyList::empty(py);
            for item in items {
                out.append(to_python(py, item, decimal)?)?;
            }
            Ok(out.into_any())
        }
        SemanticValue::Mapping(entries) => {
            let out = PyDict::new(py);
            for (key, value) in entries {
                out.set_item(key, to_python(py, value, decimal)?)?;
            }
            Ok(out.into_any())
        }
    }
}

pub(crate) fn tags(
    node: &SemanticNode,
    pointer: &mut String,
    out: &Bound<'_, PyDict>,
) -> PyResult<()> {
    if let Some(tag) = &node.tag {
        out.set_item(pointer.as_str(), &tag.name)?;
    }
    let parent = pointer.len();
    match &node.value {
        SemanticValue::Sequence(items) => {
            for (index, item) in items.iter().enumerate() {
                write!(pointer, "/{index}").expect("writing to a String cannot fail");
                tags(item, pointer, out)?;
                pointer.truncate(parent);
            }
        }
        SemanticValue::Mapping(entries) => {
            for (key, value) in entries {
                push_pointer_key(pointer, key);
                tags(value, pointer, out)?;
                pointer.truncate(parent);
            }
        }
        _ => {}
    }
    Ok(())
}
