use daml_core::{self as core, SemanticNode};
use pyo3::{
    prelude::*,
    types::{PyList, PyTuple},
};

use crate::{semantic, source_edit_exception};

// Only newly supplied paths and values are owned here. Original source and tree
// remain borrowed from the frozen Python document throughout preparation/apply.
enum Edit {
    Replace { path: String, value: SemanticNode },
    Insert { path: String, value: SemanticNode },
    Remove { path: String },
}

pub(crate) fn edit(
    py: Python<'_>,
    document: &core::Document,
    edits: &Bound<'_, PyAny>,
    context: Option<(&core::Schemas, Option<&Bound<'_, PyAny>>)>,
    line_width: usize,
) -> PyResult<core::Document> {
    let count = if let Ok(list) = edits.cast::<PyList>() {
        list.len()
    } else {
        edits
            .cast::<PyTuple>()
            .map_err(|_| {
                semantic::invalid(
                    py,
                    "InvalidInput",
                    "source edits must be a list or tuple",
                    "",
                )
            })?
            .len()
    };
    let limits = core::SourceEditLimits::default();
    if count > limits.max_operations {
        return Err(source_edit_exception(
            py,
            &core::SourceEditError::LimitExceeded,
        ));
    }

    // One ledger covers the complete wire batch, including metadata and expected.
    // Admission precedes owned allocation; it never resets for individual values.
    let mut budget = semantic::Budget::default();
    budget.charge(py, count, 0, "")?;
    let (schemas, expected) = match context {
        Some((schemas, expected)) => (
            Some(schemas),
            expected
                .map(|expected| budget.borrow_string(expected, ""))
                .transpose()?,
        ),
        None => (None, None),
    };
    let mut admitted = Vec::with_capacity(count);
    for index in 0..count {
        let edit = if let Ok(list) = edits.cast::<PyList>() {
            list.get_item(index)?
        } else {
            edits.cast::<PyTuple>()?.get_item(index)?
        };
        let record = semantic::fields(&edit, &["op", "path", "value"], "")?;
        let op = semantic::required(record, "op", "")?;
        let op = semantic::text(&op, 16, "")?;
        budget.charge(py, 0, op.len(), "")?;
        let allowed: &[&str] = match op {
            "replace" | "insert" => &["op", "path", "value"],
            "remove" => &["op", "path"],
            _ => {
                return Err(semantic::invalid(
                    py,
                    "InvalidInput",
                    "unknown source edit operation",
                    "",
                ));
            }
        };
        semantic::fields(&edit, allowed, "")?;
        let path = budget.string(&semantic::required(record, "path", "")?, "")?;
        admitted.push(match op {
            "replace" => Edit::Replace {
                value: semantic::decode_with_budget(
                    &semantic::required(record, "value", &path)?,
                    &mut budget,
                )?,
                path,
            },
            "insert" => Edit::Insert {
                value: semantic::decode_with_budget(
                    &semantic::required(record, "value", &path)?,
                    &mut budget,
                )?,
                path,
            },
            "remove" => Edit::Remove { path },
            _ => unreachable!("source edit operations were admitted above"),
        });
    }

    // No Python objects or borrowed patches cross this detached one-shot call.
    py.detach(|| {
        let edits: Vec<_> = admitted
            .iter()
            .map(|edit| match edit {
                Edit::Replace { path, value } => core::SourceEdit::Replace { path, value },
                Edit::Insert { path, value } => core::SourceEdit::Insert { path, value },
                Edit::Remove { path } => core::SourceEdit::Remove { path },
            })
            .collect();
        let options = core::FormatOptions { line_width };
        let patch = match schemas {
            Some(schemas) => {
                schemas.prepare_source_edits(document, expected, &edits, options, limits)?
            }
            None => document.prepare_source_edits(&edits, options, limits)?,
        };
        patch.apply(document.source())
    })
    .map_err(|error| source_edit_exception(py, &error))
}
