use std::{error::Error, fmt::Write as _, path::PathBuf, sync::Arc};

use daml_core::{self as core, ErrorOrigin, Node, ResolvedValue, Span, Value};
use pyo3::{
    create_exception,
    exceptions::{PyOSError, PyValueError},
    prelude::*,
    types::{PyBool, PyDict, PyInt, PyList, PyString},
};
use serde_json::Value as JsonValue;

create_exception!(daml, ParseError, PyValueError);
create_exception!(daml, NativeError, PyValueError);
create_exception!(daml, SchemaError, PyValueError);
create_exception!(daml, ResolveError, PyValueError);

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

fn json_to_python<'py>(
    py: Python<'py>,
    value: &JsonValue,
    decimal: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    match value {
        JsonValue::Null => Ok(py.None().into_bound(py)),
        JsonValue::Bool(value) => Ok(PyBool::new(py, *value).to_owned().into_any()),
        JsonValue::Number(value) => number_to_python(py, value.as_str(), decimal),
        JsonValue::String(value) => Ok(PyString::new(py, value).into_any()),
        JsonValue::Array(values) => {
            let list = PyList::empty(py);
            for child in values {
                list.append(json_to_python(py, child, decimal)?)?;
            }
            Ok(list.into_any())
        }
        JsonValue::Object(values) => {
            let dict = PyDict::new(py);
            for (key, child) in values {
                dict.set_item(key, json_to_python(py, child, decimal)?)?;
            }
            Ok(dict.into_any())
        }
    }
}

fn resolved_tags(
    node: &core::ResolvedNode<'_>,
    pointer: &mut String,
    tags: &mut Vec<(String, String)>,
) {
    if let Some(tag) = node.tag() {
        tags.push((pointer.clone(), tag.to_owned()));
    }
    let parent_len = pointer.len();
    match node.value() {
        ResolvedValue::Sequence(items) => {
            for (index, child) in items.iter().enumerate() {
                write!(pointer, "/{index}").expect("writing into a String cannot fail");
                resolved_tags(child, pointer, tags);
                pointer.truncate(parent_len);
            }
        }
        ResolvedValue::Mapping(fields) => {
            for (key, child) in fields {
                push_pointer_key(pointer, key);
                resolved_tags(child, pointer, tags);
                pointer.truncate(parent_len);
            }
        }
        _ => {}
    }
}

fn tags_dict<'py>(py: Python<'py>, tags: &[(String, String)]) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    for (pointer, tag) in tags {
        result.set_item(pointer, tag)?;
    }
    Ok(result)
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
    fn discover(py: Python<'_>, document_path: PathBuf) -> PyResult<Option<Self>> {
        py.detach(|| core::Bundle::discover(document_path))
            .map(|bundle| bundle.map(|inner| Self { inner }))
            .map_err(|error| native_exception(py, &error))
    }

    #[pyo3(signature = (document, expected = None))]
    fn compile(
        &self,
        py: Python<'_>,
        document: PyRef<'_, PyDocument>,
        expected: Option<&str>,
    ) -> PyResult<PySchemas> {
        let bundle = &self.inner;
        let document = &document.inner;
        let schemas = py.detach(|| bundle.compile(document, expected));
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
            let mut tags = Vec::new();
            resolved_tags(resolved.root(), &mut String::new(), &mut tags);
            Ok((resolved.to_json(), tags))
        });
        resolved
            .map(|(snapshot, tags)| PyResolvedDocument { snapshot, tags })
            .map_err(|error| resolve_exception(py, &error))
    }
}

#[pyclass(frozen, name = "ResolvedDocument", module = "daml")]
struct PyResolvedDocument {
    snapshot: JsonValue,
    tags: Vec<(String, String)>,
}

#[pymethods]
impl PyResolvedDocument {
    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let decimal = py.import("decimal")?.getattr("Decimal")?;
        json_to_python(py, &self.snapshot, &decimal).map(Bound::unbind)
    }

    fn tags(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        tags_dict(py, &self.tags).map(Bound::unbind)
    }
}

#[pyfunction]
fn parse(py: Python<'_>, source: &str) -> PyResult<PyDocument> {
    py.detach(|| core::parse(source))
        .map(|inner| PyDocument { inner })
        .map_err(|error| parse_exception(py, &error))
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
    module.add_function(wrap_pyfunction!(parse, module)?)?;
    module.add_function(wrap_pyfunction!(format_definition, module)?)?;
    Ok(())
}
