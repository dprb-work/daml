use daml_core::{
    AdmissionStage, CopyEdge, ErrorOrigin, NodeSummary, ResolveError, ResolveErrorKind,
    ResolvedNode, ResolvedValue, SemanticNode, StageEdit, StageError, StageOutcome, SummaryValue,
};
use pyo3::{
    exceptions::{PyAttributeError, PyBaseException},
    prelude::*,
    types::{PyBool, PyBytes, PyDict, PyInt, PyList, PyTuple},
};

use crate::{PyResolvedDocument, resolve_exception, semantic};

// This budget covers binding-owned work across every round and candidate replay.
// The core independently bounds its own completion, admission and retained state.
pub(crate) struct Stage<'py> {
    py: Python<'py>,
    defer: Bound<'py, PyAny>,
    materialize: Bound<'py, PyAny>,
    number_class: Option<Bound<'py, PyAny>>,
    budget: semantic::Budget,
}

// Translate only SDK-owned wire budget failures. Exceptions raised by user methods
// bypass this conversion and retain their original Python identity and traceback.
fn wire_error(py: Python<'_>, error: PyErr) -> PyErr {
    if error.is_instance_of::<crate::SemanticError>(py)
        && error
            .value(py)
            .getattr("kind")
            .and_then(|kind| kind.extract::<String>())
            .is_ok_and(|kind| kind == "LimitExceeded")
    {
        return resolve_exception(
            py,
            &ResolveError {
                kind: ResolveErrorKind::LimitExceeded,
                path: String::new(),
                origin: ErrorOrigin::Semantic,
                schema: None,
                message: "staged callback work exceeds cumulative bounds".to_owned(),
            },
        );
    }
    error
}

pub(crate) fn exception(py: Python<'_>, error: StageError<PyErr>) -> PyErr {
    match error {
        StageError::Resolve(error) => resolve_exception(py, &error),
        StageError::Callback(error) | StageError::Rejected(error) => error,
    }
}

impl<'py> Stage<'py> {
    pub(crate) fn new(py: Python<'py>, stage: &Bound<'py, PyAny>) -> PyResult<Self> {
        let defer = stage.getattr("defer")?;
        let materialize = stage.getattr("materialize")?;
        let number_class = match stage.getattr("number_class") {
            Ok(method) => Some(method),
            Err(error) if error.is_instance_of::<PyAttributeError>(py) => None,
            Err(error) => return Err(error),
        };
        if !defer.is_callable()
            || !materialize.is_callable()
            || number_class
                .as_ref()
                .is_some_and(|method| !method.is_callable())
        {
            return Err(semantic::invalid(
                py,
                "InvalidInput",
                "stage methods must be callable",
                "",
            ));
        }
        Ok(Self {
            py,
            defer,
            materialize,
            number_class,
            budget: semantic::Budget::default(),
        })
    }

    pub(crate) fn decode(&mut self, value: &Bound<'_, PyAny>) -> PyResult<SemanticNode> {
        semantic::decode_with_budget(value, &mut self.budget)
    }

    pub(crate) fn borrow_string<'a>(
        &mut self,
        value: &'a Bound<'_, PyAny>,
        pointer: &str,
    ) -> PyResult<&'a str> {
        self.budget.borrow_string(value, pointer)
    }

    pub(crate) fn edits(
        &mut self,
        value: &Bound<'_, PyAny>,
        initial: bool,
    ) -> PyResult<Vec<StageEdit>> {
        let edits = self.decode_edits(value, initial);
        if initial {
            edits
        } else {
            edits.map_err(|error| wire_error(self.py, error))
        }
    }

    fn decode_edits(
        &mut self,
        value: &Bound<'_, PyAny>,
        initial: bool,
    ) -> PyResult<Vec<StageEdit>> {
        let count = if let Ok(list) = value.cast::<PyList>() {
            list.len()
        } else if initial {
            value
                .cast::<PyTuple>()
                .map_err(|_| {
                    semantic::invalid(
                        self.py,
                        "InvalidInput",
                        "initial edits must be a list or tuple",
                        "",
                    )
                })?
                .len()
        } else {
            return Err(semantic::invalid(
                self.py,
                "InvalidInput",
                "stage edits must be a list",
                "",
            ));
        };
        self.budget.charge(self.py, count, 0, "")?;
        let mut edits = Vec::with_capacity(count);
        for index in 0..count {
            let edit = if let Ok(list) = value.cast::<PyList>() {
                list.get_item(index)?
            } else {
                value.cast::<PyTuple>()?.get_item(index)?
            };
            let record = semantic::fields(&edit, &["op", "path", "value", "from", "name"], "")?;
            let op = semantic::required(record, "op", "")?;
            let op = semantic::text(&op, 16, "")?;
            let allowed: &[&str] = match op {
                "replace" => &["op", "path", "value"],
                "copy" => &["op", "path", "from"],
                "expect" => &["op", "path", "name"],
                _ => {
                    return Err(semantic::invalid(
                        self.py,
                        "InvalidInput",
                        "unknown stage edit operation",
                        "",
                    ));
                }
            };
            semantic::fields(&edit, allowed, "")?;
            let path = self
                .budget
                .string(&semantic::required(record, "path", "")?, "")?;
            edits.push(match op {
                "replace" => StageEdit::Replace {
                    value: semantic::decode_with_budget(
                        &semantic::required(record, "value", &path)?,
                        &mut self.budget,
                    )?,
                    path,
                },
                "copy" => StageEdit::Copy {
                    from: self
                        .budget
                        .string(&semantic::required(record, "from", &path)?, &path)?,
                    path,
                },
                "expect" => StageEdit::Expect {
                    name: self
                        .budget
                        .string(&semantic::required(record, "name", &path)?, &path)?,
                    path,
                },
                _ => unreachable!("stage edit operations were admitted above"),
            });
        }
        Ok(edits)
    }

    pub(crate) fn finish(
        &mut self,
        root: &ResolvedNode<'_>,
        meaning: Option<&[u8]>,
    ) -> PyResult<PyResolvedDocument> {
        charge_resolved(self.py, root, &mut self.budget)
            .map_err(|error| wire_error(self.py, error))?;
        if let Some(meaning) = meaning {
            self.budget
                .charge(self.py, 0, meaning.len(), "")
                .map_err(|error| wire_error(self.py, error))?;
        }
        Ok(PyResolvedDocument {
            snapshot: SemanticNode::from_resolved(root),
            meaning: meaning.map(<[u8]>::to_vec),
        })
    }
}

fn charge_value(
    py: Python<'_>,
    value: SummaryValue<'_>,
    budget: &mut semantic::Budget,
) -> PyResult<()> {
    let bytes = match value {
        SummaryValue::Number(value) | SummaryValue::String(value) => value.len(),
        _ => 0,
    };
    budget.charge(py, 1, bytes, "")
}

fn charge_tag(
    py: Python<'_>,
    name: Option<&str>,
    binding: Option<&str>,
    budget: &mut semantic::Budget,
) -> PyResult<()> {
    if let Some(name) = name {
        budget.charge(py, 1, name.len(), "")?;
        if let Some(binding) = binding {
            budget.charge(py, 0, binding.len(), "")?;
        }
    }
    Ok(())
}

fn shallow_value<'py>(py: Python<'py>, value: SummaryValue<'_>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    let kind = match value {
        SummaryValue::Null => "null",
        SummaryValue::Bool(value) => {
            out.set_item("value", value)?;
            "bool"
        }
        SummaryValue::Number(value) => {
            out.set_item("value", value)?;
            "number"
        }
        SummaryValue::String(value) => {
            out.set_item("value", value)?;
            "string"
        }
        SummaryValue::Sequence(length) => {
            out.set_item("length", length)?;
            "list"
        }
        SummaryValue::Mapping(length) => {
            out.set_item("length", length)?;
            "map"
        }
    };
    out.set_item("kind", kind)?;
    Ok(out)
}

fn export_tag(out: &Bound<'_, PyDict>, name: Option<&str>, binding: Option<&str>) -> PyResult<()> {
    if let Some(name) = name {
        let tag = PyDict::new(out.py());
        tag.set_item("name", name)?;
        if let Some(binding) = binding {
            tag.set_item("binding", binding)?;
        }
        out.set_item("tag", tag)?;
    }
    Ok(())
}

fn charge_resolved(
    py: Python<'_>,
    node: &ResolvedNode<'_>,
    budget: &mut semantic::Budget,
) -> PyResult<()> {
    let value = match node.value() {
        ResolvedValue::Null => SummaryValue::Null,
        ResolvedValue::Bool(value) => SummaryValue::Bool(*value),
        ResolvedValue::Number(value) => SummaryValue::Number(value.as_str()),
        ResolvedValue::String(value) => SummaryValue::String(value),
        ResolvedValue::Sequence(items) => SummaryValue::Sequence(items.len()),
        ResolvedValue::Mapping(entries) => SummaryValue::Mapping(entries.len()),
    };
    charge_value(py, value, budget)?;
    charge_tag(py, node.tag(), node.tag_binding(), budget)?;
    match node.value() {
        ResolvedValue::Sequence(items) => {
            for item in items {
                charge_resolved(py, item, budget)?;
            }
        }
        ResolvedValue::Mapping(entries) => {
            for (key, value) in entries {
                budget.charge(py, 1, key.len(), "")?;
                charge_resolved(py, value, budget)?;
            }
        }
        _ => {}
    }
    Ok(())
}

// Charge each part before its allocation. A failed partial export is private and
// dropped; user mutation cannot alter the resolver's borrowed candidate.
fn export_resolved<'py>(
    py: Python<'py>,
    node: &ResolvedNode<'_>,
    budget: &mut semantic::Budget,
) -> PyResult<Bound<'py, PyDict>> {
    let value = match node.value() {
        ResolvedValue::Null => SummaryValue::Null,
        ResolvedValue::Bool(value) => SummaryValue::Bool(*value),
        ResolvedValue::Number(value) => SummaryValue::Number(value.as_str()),
        ResolvedValue::String(value) => SummaryValue::String(value),
        ResolvedValue::Sequence(items) => SummaryValue::Sequence(items.len()),
        ResolvedValue::Mapping(entries) => SummaryValue::Mapping(entries.len()),
    };
    charge_value(py, value, budget)?;
    charge_tag(py, node.tag(), node.tag_binding(), budget)?;
    let out = PyDict::new(py);
    let kind = match node.value() {
        ResolvedValue::Null => "null",
        ResolvedValue::Bool(value) => {
            out.set_item("value", value)?;
            "bool"
        }
        ResolvedValue::Number(value) => {
            out.set_item("value", value.as_str())?;
            "number"
        }
        ResolvedValue::String(value) => {
            out.set_item("value", value.as_ref())?;
            "string"
        }
        ResolvedValue::Sequence(items) => {
            let output = PyList::empty(py);
            for item in items {
                output.append(export_resolved(py, item, budget)?)?;
            }
            out.set_item("items", output)?;
            "list"
        }
        ResolvedValue::Mapping(entries) => {
            let output = PyList::empty(py);
            for (key, value) in entries {
                budget.charge(py, 1, key.len(), "")?;
                let entry = PyDict::new(py);
                entry.set_item("key", key.as_ref())?;
                entry.set_item("value", export_resolved(py, value, budget)?)?;
                output.append(entry)?;
            }
            out.set_item("entries", output)?;
            "map"
        }
    };
    out.set_item("kind", kind)?;
    export_tag(&out, node.tag(), node.tag_binding())?;
    Ok(out)
}

impl AdmissionStage for Stage<'_> {
    type Error = PyErr;

    fn defer(&mut self, node: NodeSummary<'_>, path: &str) -> PyResult<bool> {
        let py = self.py;
        let summary = (|| {
            self.budget.charge(py, 0, path.len(), path)?;
            charge_value(py, node.value(), &mut self.budget)?;
            charge_tag(py, node.tag(), node.tag_binding(), &mut self.budget)?;
            for (key, value) in node.fields() {
                self.budget.charge(py, 1, key.len(), path)?;
                charge_value(py, value, &mut self.budget)?;
            }
            let summary = shallow_value(py, node.value())?;
            export_tag(&summary, node.tag(), node.tag_binding())?;
            if matches!(node.value(), SummaryValue::Mapping(_)) {
                let fields = PyList::empty(py);
                for (key, value) in node.fields() {
                    let field = PyDict::new(py);
                    field.set_item("key", key)?;
                    field.set_item("summary", shallow_value(py, value)?)?;
                    fields.append(field)?;
                }
                summary.set_item("fields", fields)?;
            }
            Ok(summary)
        })()
        .map_err(|error| wire_error(py, error))?;
        let result = self.defer.call1((summary, path))?;
        if !result.is_instance_of::<PyBool>() {
            return Err(semantic::invalid(
                py,
                "InvalidInput",
                "defer must return a boolean",
                path,
            ));
        }
        result.extract()
    }

    fn materialize(
        &mut self,
        root: &ResolvedNode<'_>,
        copies: &[CopyEdge],
        round: usize,
    ) -> PyResult<StageOutcome<PyErr>> {
        let py = self.py;
        let (value, history) = (|| {
            let value = export_resolved(py, root, &mut self.budget)?;
            self.budget.charge(py, copies.len(), 0, "")?;
            for copy in copies {
                self.budget.charge(py, 0, copy.path.len(), "")?;
                self.budget.charge(py, 0, copy.from.len(), "")?;
            }
            let history = PyList::empty(py);
            for copy in copies {
                let edge = PyDict::new(py);
                edge.set_item("path", &copy.path)?;
                edge.set_item("from", &copy.from)?;
                history.append(edge)?;
            }
            Ok((value, history))
        })()
        .map_err(|error| wire_error(py, error))?;
        let result = self.materialize.call1((value, history, round))?;
        if result.is_none() {
            return Ok(StageOutcome::Ready { meaning: None });
        }
        if let Ok(bytes) = result.cast::<PyBytes>() {
            let bytes = bytes.as_bytes();
            self.budget
                .charge(py, 0, bytes.len(), "")
                .map_err(|error| wire_error(py, error))?;
            return Ok(StageOutcome::Ready {
                meaning: Some(bytes.to_vec()),
            });
        }
        if result.is_instance_of::<PyBaseException>() {
            return Ok(StageOutcome::Reject(PyErr::from_value(result)));
        }
        Ok(StageOutcome::Edits(self.edits(&result, false)?))
    }

    fn number_class(&mut self, lexeme: &str) -> PyResult<u32> {
        let Some(method) = &self.number_class else {
            return Ok(0);
        };
        let py = self.py;
        self.budget
            .charge(py, 1, lexeme.len(), "")
            .map_err(|error| wire_error(py, error))?;
        let result = method.call1((lexeme,))?;
        if result.is_instance_of::<PyBool>() || !result.is_instance_of::<PyInt>() {
            return Err(semantic::invalid(
                py,
                "InvalidInput",
                "number_class must return a non-bool u32",
                "",
            ));
        }
        result.extract::<u32>().map_err(|_| {
            semantic::invalid(
                py,
                "InvalidInput",
                "number_class must return a non-bool u32",
                "",
            )
        })
    }
}
