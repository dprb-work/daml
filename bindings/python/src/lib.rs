use std::{error::Error, fmt::Write as _, path::PathBuf, sync::Arc};

use daml_core::{self as core, ErrorOrigin, Node, SemanticNode, Span, Value};
use pyo3::{
    create_exception,
    exceptions::{PyOSError, PyValueError},
    prelude::*,
    types::{PyBool, PyBytes, PyDict, PyInt, PyList, PyString},
};
mod semantic;
mod stage;

create_exception!(daml, ParseError, PyValueError);
create_exception!(daml, NativeError, PyValueError);
create_exception!(daml, SchemaError, PyValueError);
create_exception!(daml, ResolveError, PyValueError);
create_exception!(daml, SemanticError, PyValueError);

fn kind_name<T: std::fmt::Debug>(kind: T) -> String {
    format!("{kind:?}")
}

fn attach_source(py: Python<'_>, error: PyErr, source: Option<&(dyn Error + 'static)>) -> PyErr {
    if let Some(source) = source {
        let cause = native_cause(py, source);
        error.set_cause(py, Some(cause));
    }
    error
}

fn native_cause(py: Python<'_>, source: &(dyn Error + 'static)) -> PyErr {
    if let Some(error) = source.downcast_ref::<core::ParseError>() {
        return parse_exception(py, error);
    }
    if let Some(error) = source.downcast_ref::<core::NativeError>() {
        return native_exception(py, error);
    }
    if let Some(error) = source.downcast_ref::<core::SchemaError>() {
        return schema_exception(py, error);
    }
    if let Some(error) = source.downcast_ref::<core::ResolveError>() {
        return resolve_exception(py, error);
    }
    if let Some(error) = source.downcast_ref::<core::SemanticError>() {
        return semantic_exception(py, error);
    }
    if let Some(error) = source.downcast_ref::<std::io::Error>() {
        let result = PyOSError::new_err((error.raw_os_error(), error.to_string()));
        return attach_source(py, result, error.source());
    }

    let error = PyValueError::new_err(source.to_string());
    if let Some(next) = source.source() {
        error.set_cause(py, Some(native_cause(py, next)));
    }
    error
}
macro_rules! set_exception_attr {
    ($py:expr, $error:expr, $name:literal, $value:expr) => {
        if let Err(set_error) = $error.value($py).setattr($name, $value) {
            return set_error;
        }
    };
}

fn span_tuple(span: Span) -> (usize, usize) {
    (span.start, span.end)
}

fn parse_exception(py: Python<'_>, error: &core::ParseError) -> PyErr {
    let result = PyErr::from_type(py.get_type::<ParseError>(), error.to_string());
    set_exception_attr!(py, result, "kind", kind_name(error.kind));
    set_exception_attr!(py, result, "span", span_tuple(error.span));
    set_exception_attr!(py, result, "path", py.None());
    result
}

fn native_exception(py: Python<'_>, error: &core::NativeError) -> PyErr {
    let result = PyErr::from_type(py.get_type::<NativeError>(), error.to_string());
    set_exception_attr!(py, result, "kind", kind_name(error.kind));
    set_exception_attr!(py, result, "span", error.span.map(span_tuple));
    set_exception_attr!(
        py,
        result,
        "path",
        error.path.as_deref().map(|path| path.as_os_str())
    );
    set_exception_attr!(py, result, "reference", error.reference.as_deref());
    attach_source(py, result, error.source())
}

fn schema_exception(py: Python<'_>, error: &core::SchemaError) -> PyErr {
    let result = PyErr::from_type(py.get_type::<SchemaError>(), error.to_string());
    set_exception_attr!(py, result, "kind", kind_name(error.kind));
    set_exception_attr!(
        py,
        result,
        "schema",
        error
            .location
            .as_ref()
            .map(|location| (location.resource.as_str(), location.pointer.as_str()))
    );
    attach_source(py, result, error.source())
}

fn semantic_exception(py: Python<'_>, error: &core::SemanticError) -> PyErr {
    semantic::invalid(py, &kind_name(error.kind), &error.message, &error.pointer)
}

fn resolve_exception(py: Python<'_>, error: &core::ResolveError) -> PyErr {
    let result = PyErr::from_type(py.get_type::<ResolveError>(), error.to_string());
    set_exception_attr!(py, result, "kind", kind_name(error.kind));
    set_exception_attr!(py, result, "path", error.path.as_str());
    set_exception_attr!(
        py,
        result,
        "schema",
        error
            .schema
            .as_ref()
            .map(|location| (location.resource.as_str(), location.pointer.as_str()))
    );
    match &error.origin {
        ErrorOrigin::Input(span) => {
            set_exception_attr!(py, result, "span", span_tuple(*span));
            set_exception_attr!(py, result, "origin", py.None());
        }
        ErrorOrigin::Semantic => {
            set_exception_attr!(py, result, "span", py.None());
            set_exception_attr!(py, result, "origin", py.None());
        }
        ErrorOrigin::Default(location) => {
            set_exception_attr!(py, result, "span", py.None());
            set_exception_attr!(
                py,
                result,
                "origin",
                (location.resource.as_str(), location.pointer.as_str())
            );
        }
    }
    result
}

fn number_to_python<'py>(
    py: Python<'py>,
    raw: &str,
    decimal: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    if raw.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E')) {
        decimal.call1((raw,))
    } else {
        py.get_type::<PyInt>().call1((raw,))
    }
}

fn source_to_python<'py>(
    py: Python<'py>,
    document: &core::Document,
    node: &Node,
    decimal: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    match &node.value {
        Value::Null => Ok(py.None().into_bound(py)),
        Value::Bool(value) => Ok(PyBool::new(py, *value).to_owned().into_any()),
        Value::Number(span) => number_to_python(
            py,
            document
                .slice(*span)
                .expect("parsed document owns its numeric spans"),
            decimal,
        ),
        Value::String(_) => Ok(PyString::new(
            py,
            document
                .string(node)
                .expect("parsed document owns its string nodes"),
        )
        .into_any()),
        Value::Sequence(items) => {
            let list = PyList::empty(py);
            for child in items {
                list.append(source_to_python(py, document, child, decimal)?)?;
            }
            Ok(list.into_any())
        }
        Value::Mapping(entries) => {
            let dict = PyDict::new(py);
            for entry in entries {
                let key = document
                    .slice(entry.key)
                    .expect("parsed document owns its key spans");
                dict.set_item(key, source_to_python(py, document, &entry.value, decimal)?)?;
            }
            Ok(dict.into_any())
        }
    }
}

fn source_tags(
    node: &Node,
    source: &str,
    pointer: &mut String,
    tags: &Bound<'_, PyDict>,
) -> PyResult<()> {
    if let Some(span) = node.tag {
        tags.set_item(pointer.as_str(), &source[span.start..span.end])?;
    }
    let parent_len = pointer.len();
    match &node.value {
        Value::Sequence(items) => {
            for (index, child) in items.iter().enumerate() {
                write!(pointer, "/{index}").expect("writing into a String cannot fail");
                source_tags(child, source, pointer, tags)?;
                pointer.truncate(parent_len);
            }
        }
        Value::Mapping(entries) => {
            for entry in entries {
                push_pointer_key(pointer, &source[entry.key.start..entry.key.end]);
                source_tags(&entry.value, source, pointer, tags)?;
                pointer.truncate(parent_len);
            }
        }
        _ => {}
    }
    Ok(())
}

fn push_pointer_key(pointer: &mut String, key: &str) {
    pointer.push('/');
    for character in key.chars() {
        match character {
            '~' => pointer.push_str("~0"),
            '/' => pointer.push_str("~1"),
            character => pointer.push(character),
        }
    }
}

#[pyclass(frozen, name = "Document", module = "daml")]
struct PyDocument {
    inner: core::Document,
}

#[pymethods]
impl PyDocument {
    #[getter]
    fn source(&self) -> &str {
        self.inner.source()
    }

    fn value(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        semantic::document(py, &self.inner, self.inner.root()).map(Bound::unbind)
    }

    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let decimal = py.import("decimal")?.getattr("Decimal")?;
        source_to_python(py, &self.inner, self.inner.root(), &decimal).map(Bound::unbind)
    }
    fn tags(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let tags = PyDict::new(py);
        source_tags(
            self.inner.root(),
            self.inner.source(),
            &mut String::new(),
            &tags,
        )?;
        Ok(tags.unbind())
    }

    #[pyo3(signature = (line_width = 80))]
    fn format(&self, py: Python<'_>, line_width: usize) -> PyResult<String> {
        py.detach(|| core::format(&self.inner, core::FormatOptions { line_width }))
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }
}

#[pyclass(frozen, name = "SchemaModule", module = "daml")]
struct PySchemaModule {
    inner: Arc<core::SchemaModule>,
}

#[pymethods]
impl PySchemaModule {
    #[staticmethod]
    fn from_json(py: Python<'_>, source: &str) -> PyResult<Self> {
        py.detach(|| core::SchemaModule::from_json(source))
            .map(|inner| Self {
                inner: Arc::new(inner),
            })
            .map_err(|error| schema_exception(py, &error))
    }

    #[staticmethod]
    fn from_daml(py: Python<'_>, source: &str) -> PyResult<Self> {
        py.detach(|| core::SchemaModule::from_daml(source))
            .map(|inner| Self {
                inner: Arc::new(inner),
            })
            .map_err(|error| schema_exception(py, &error))
    }
}

#[pyclass(frozen, name = "Bundle", module = "daml")]
struct PyBundle {
    inner: core::Bundle,
}

#[pymethods]
impl PyBundle {
    #[staticmethod]
    fn open(py: Python<'_>, root: PathBuf) -> PyResult<Self> {
        py.detach(|| core::Bundle::open(root))
            .map(|inner| Self { inner })
            .map_err(|error| native_exception(py, &error))
    }

    #[staticmethod]
    fn from_sources(
        py: Python<'_>,
        root: &Bound<'_, PyAny>,
        sources: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        let root = semantic::text(root, 4096, "")?;
        let sources = sources.cast::<PyDict>().map_err(|_| {
            semantic::invalid(py, "InvalidInput", "sources must be a dictionary", "")
        })?;
        if sources.len() > 10_000 {
            return Err(semantic::invalid(
                py,
                "LimitExceeded",
                "source table exceeds resource count bound",
                "",
            ));
        }
        let mut admitted = Vec::with_capacity(sources.len());
        let mut remaining = 8 * 1024 * 1024;
        for (key, source) in sources.iter() {
            let key = semantic::text(&key, 4096.min(remaining), "")?;
            remaining -= key.len();
            let source = semantic::text(&source, remaining, key)?;
            remaining -= source.len();
            admitted.push((key.to_owned(), source.to_owned()));
        }
        py.detach(|| core::Bundle::from_sources(root, admitted))
            .map(|inner| Self { inner })
            .map_err(|error| native_exception(py, &error))
    }

    #[staticmethod]
    fn discover(py: Python<'_>, document_path: PathBuf) -> PyResult<Option<Self>> {
        py.detach(|| core::Bundle::discover(document_path))
            .map(|bundle| bundle.map(|inner| Self { inner }))
            .map_err(|error| native_exception(py, &error))
    }

    #[pyo3(signature = (document, expected = None, additional_types = None))]
    fn compile(
        &self,
        py: Python<'_>,
        document: PyRef<'_, PyDocument>,
        expected: Option<&str>,
        additional_types: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PySchemas> {
        let mut names = Vec::new();
        let mut remaining = 8 * 1024 * 1024;
        if let Some(additional_types) = additional_types {
            for name in additional_types.try_iter()? {
                if names.len() >= 10_000 {
                    return Err(semantic::invalid(
                        py,
                        "LimitExceeded",
                        "additional types exceed name count bound",
                        "",
                    ));
                }
                let name = name?;
                let name = semantic::text(&name, (1024 * 1024).min(remaining), "")?;
                remaining -= name.len();
                names.push(name.to_owned());
            }
        }
        let bundle = &self.inner;
        let document = &document.inner;
        let schemas = py.detach(|| {
            bundle.compile_with_types(document, expected, names.iter().map(String::as_str))
        });
        schemas
            .map(|inner| PySchemas { inner })
            .map_err(|error| native_exception(py, &error))
    }
}

#[pyclass(frozen, name = "Schemas", module = "daml")]
struct PySchemas {
    inner: core::Schemas,
}

#[pymethods]
impl PySchemas {
    #[staticmethod]
    fn compose(py: Python<'_>, modules: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut admitted = Vec::new();
        for module in modules.try_iter()? {
            let module = module?;
            let module = module.extract::<PyRef<'_, PySchemaModule>>()?;
            admitted.push(Arc::clone(&module.inner));
        }
        py.detach(|| core::Schemas::compose(admitted.into_iter().map(|module| (*module).clone())))
            .map(|inner| Self { inner })
            .map_err(|error| schema_exception(py, &error))
    }

    #[pyo3(signature = (document, expected = None))]
    fn resolve(
        &self,
        py: Python<'_>,
        document: PyRef<'_, PyDocument>,
        expected: Option<&str>,
    ) -> PyResult<PyResolvedDocument> {
        let schemas = &self.inner;
        let document = &document.inner;
        let resolved = py.detach(|| {
            let resolved = match expected {
                Some(expected) => schemas.resolve_as(document, expected)?,
                None => schemas.resolve(document)?,
            };
            Ok(SemanticNode::from_resolved(resolved.root()))
        });
        resolved
            .map(|snapshot| PyResolvedDocument {
                snapshot,
                meaning: None,
            })
            .map_err(|error| resolve_exception(py, &error))
    }

    #[pyo3(signature = (value, expected = None))]
    fn resolve_semantic(
        &self,
        py: Python<'_>,
        value: &Bound<'_, PyAny>,
        expected: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyResolvedDocument> {
        let mut budget = semantic::Budget::default();
        let expected = expected
            .map(|expected| budget.borrow_string(expected, ""))
            .transpose()?;
        let node = semantic::decode_with_budget(value, &mut budget)?;
        py.detach(|| {
            self.inner
                .resolve_semantic(&node, expected)
                .map(|resolved| PyResolvedDocument {
                    snapshot: SemanticNode::from_resolved(&resolved),
                    meaning: None,
                })
        })
        .map_err(|error| resolve_exception(py, &error))
    }

    #[pyo3(
        signature = (document, stage, expected = None, initial_edits = None),
        text_signature = "($self, document, stage, expected=None, initial_edits=None)"
    )]
    fn resolve_staged<'py>(
        &self,
        py: Python<'py>,
        document: PyRef<'_, PyDocument>,
        stage: &Bound<'py, PyAny>,
        expected: Option<&Bound<'py, PyAny>>,
        initial_edits: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<PyResolvedDocument> {
        let mut stage = stage::Stage::new(py, stage)?;
        let expected = expected
            .map(|expected| stage.borrow_string(expected, ""))
            .transpose()?;
        let initial_edits = match initial_edits {
            Some(edits) => stage.edits(edits, true)?,
            None => Vec::new(),
        };
        let resolved = self
            .inner
            .resolve_staged(&document.inner, expected, &initial_edits, &mut stage)
            .map_err(|error| stage::exception(py, error))?;
        stage.finish(resolved.root(), resolved.meaning())
    }

    #[pyo3(
        signature = (value, stage, expected = None, initial_edits = None),
        text_signature = "($self, value, stage, expected=None, initial_edits=None)"
    )]
    fn resolve_semantic_staged<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
        stage: &Bound<'py, PyAny>,
        expected: Option<&Bound<'py, PyAny>>,
        initial_edits: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<PyResolvedDocument> {
        let mut stage = stage::Stage::new(py, stage)?;
        let expected = expected
            .map(|expected| stage.borrow_string(expected, ""))
            .transpose()?;
        let node = stage.decode(value)?;
        let initial_edits = match initial_edits {
            Some(edits) => stage.edits(edits, true)?,
            None => Vec::new(),
        };
        let resolved = self
            .inner
            .resolve_semantic_staged(&node, expected, &initial_edits, &mut stage)
            .map_err(|error| stage::exception(py, error))?;
        stage.finish(resolved.root(), resolved.meaning())
    }

    #[pyo3(signature = (value, line_width = 80))]
    fn write(
        &self,
        py: Python<'_>,
        value: &Bound<'_, PyAny>,
        line_width: usize,
    ) -> PyResult<String> {
        let node = semantic::decode(value)?;
        py.detach(|| {
            self.inner
                .format_semantic(&node, core::FormatOptions { line_width })
        })
        .map_err(|error| semantic_exception(py, &error))
    }
}

#[pyclass(frozen, name = "ResolvedDocument", module = "daml")]
struct PyResolvedDocument {
    snapshot: SemanticNode,
    meaning: Option<Vec<u8>>,
}

#[pymethods]
impl PyResolvedDocument {
    fn meaning<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.meaning
            .as_ref()
            .map(|meaning| PyBytes::new(py, meaning))
    }

    fn value(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        semantic::encode(py, &self.snapshot).map(Bound::unbind)
    }

    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let decimal = py.import("decimal")?.getattr("Decimal")?;
        semantic::to_python(py, &self.snapshot, &decimal).map(Bound::unbind)
    }

    fn tags(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let tags = PyDict::new(py);
        semantic::tags(&self.snapshot, &mut String::new(), &tags)?;
        Ok(tags.unbind())
    }
}

#[pyfunction]
fn parse(py: Python<'_>, source: &str) -> PyResult<PyDocument> {
    py.detach(|| core::parse(source))
        .map(|inner| PyDocument { inner })
        .map_err(|error| parse_exception(py, &error))
}

#[pyfunction]
#[pyo3(signature = (value, line_width = 80))]
fn write(py: Python<'_>, value: &Bound<'_, PyAny>, line_width: usize) -> PyResult<String> {
    let node = semantic::decode(value)?;
    py.detach(|| core::format_semantic(&node, core::FormatOptions { line_width }))
        .map_err(|error| semantic_exception(py, &error))
}

#[pyfunction]
#[pyo3(signature = (source, line_width = 80))]
fn format_definition(py: Python<'_>, source: &str, line_width: usize) -> PyResult<String> {
    let output = py
        .detach(|| {
            core::parse_definition(source).map(|definition| {
                core::format_definition(&definition, core::FormatOptions { line_width })
            })
        })
        .map_err(|error| parse_exception(py, &error))?;
    output.map_err(|error| PyValueError::new_err(error.to_string()))
}

/// Native DAML parser, formatter, bundle compiler, and schema resolver.
#[pymodule]
fn daml(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyDocument>()?;
    module.add_class::<PyBundle>()?;
    module.add_class::<PySchemas>()?;
    module.add_class::<PySchemaModule>()?;
    module.add_class::<PyResolvedDocument>()?;
    module.add("ParseError", module.py().get_type::<ParseError>())?;
    module.add("NativeError", module.py().get_type::<NativeError>())?;
    module.add("SchemaError", module.py().get_type::<SchemaError>())?;
    module.add("ResolveError", module.py().get_type::<ResolveError>())?;
    module.add("SemanticError", module.py().get_type::<SemanticError>())?;
    module.add_function(wrap_pyfunction!(parse, module)?)?;
    module.add_function(wrap_pyfunction!(format_definition, module)?)?;
    module.add_function(wrap_pyfunction!(write, module)?)?;
    Ok(())
}
