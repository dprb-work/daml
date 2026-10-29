use daml_core::{TemporalKind, TemporalValue};
use pyo3::{exceptions::PyValueError, prelude::*};

fn parse(kind: TemporalKind, text: &str) -> PyResult<TemporalValue<'static>> {
    TemporalValue::parse(kind, text)
        .map(TemporalValue::into_owned)
        .map_err(|error| PyValueError::new_err(error.to_string()))
}

// Admission/storage remains entirely in the core. This opt-in boundary checks
// exact representability before asking the standard library to build its value.
fn to_stdlib<'py>(py: Python<'py>, value: &TemporalValue<'_>) -> PyResult<Bound<'py, PyAny>> {
    let text = value.as_str();
    if let Some((_, tail)) = text.split_once('.') {
        let fraction = tail
            .split(['Z', '+', '-'])
            .next()
            .expect("fraction tail exists");
        if fraction.bytes().skip(6).any(|digit| digit != b'0') {
            return Err(PyValueError::new_err(
                "temporal value has nonzero sub-microsecond precision",
            ));
        }
    }
    let class = match value.kind() {
        TemporalKind::Date => "date",
        TemporalKind::Time => "time",
        TemporalKind::DateTime => "datetime",
    };
    // fromisoformat rejects unsupported years, leap seconds and hour 24. Extra
    // fraction digits were proved zero above, so no numeric precision is lost.
    py.import("datetime")?
        .getattr(class)?
        .call_method1("fromisoformat", (text,))
}

#[derive(PartialEq)]
#[pyclass(frozen, eq, name = "Date", module = "daml")]
pub(crate) struct PyDate {
    inner: TemporalValue<'static>,
}

#[pymethods]
impl PyDate {
    #[new]
    fn new(text: &str) -> PyResult<Self> {
        Ok(Self {
            inner: parse(TemporalKind::Date, text)?,
        })
    }

    fn isoformat(&self) -> &str {
        self.inner.as_str()
    }

    fn __str__(&self) -> &str {
        self.inner.as_str()
    }

    fn __repr__(&self) -> String {
        format!("Date({:?})", self.inner.as_str())
    }

    fn to_date<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        to_stdlib(py, &self.inner)
    }
}

#[derive(PartialEq)]
#[pyclass(frozen, eq, name = "Time", module = "daml")]
pub(crate) struct PyTime {
    inner: TemporalValue<'static>,
}

#[pymethods]
impl PyTime {
    #[new]
    fn new(text: &str) -> PyResult<Self> {
        Ok(Self {
            inner: parse(TemporalKind::Time, text)?,
        })
    }

    fn isoformat(&self) -> &str {
        self.inner.as_str()
    }

    fn __str__(&self) -> &str {
        self.inner.as_str()
    }

    fn __repr__(&self) -> String {
        format!("Time({:?})", self.inner.as_str())
    }

    #[getter]
    fn offset_minutes(&self) -> Option<i16> {
        self.inner.offset_minutes()
    }

    fn to_time<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        to_stdlib(py, &self.inner)
    }
}

#[derive(PartialEq)]
#[pyclass(frozen, eq, name = "DateTime", module = "daml")]
pub(crate) struct PyDateTime {
    inner: TemporalValue<'static>,
}

#[pymethods]
impl PyDateTime {
    #[new]
    fn new(text: &str) -> PyResult<Self> {
        Ok(Self {
            inner: parse(TemporalKind::DateTime, text)?,
        })
    }

    fn isoformat(&self) -> &str {
        self.inner.as_str()
    }

    fn __str__(&self) -> &str {
        self.inner.as_str()
    }

    fn __repr__(&self) -> String {
        format!("DateTime({:?})", self.inner.as_str())
    }

    #[getter]
    fn offset_minutes(&self) -> Option<i16> {
        self.inner.offset_minutes()
    }

    fn to_datetime<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        to_stdlib(py, &self.inner)
    }
}

pub(crate) fn to_python<'py>(
    py: Python<'py>,
    value: TemporalValue<'static>,
) -> PyResult<Bound<'py, PyAny>> {
    match value.kind() {
        TemporalKind::Date => Ok(Py::new(py, PyDate { inner: value })?
            .into_bound(py)
            .into_any()),
        TemporalKind::Time => Ok(Py::new(py, PyTime { inner: value })?
            .into_bound(py)
            .into_any()),
        TemporalKind::DateTime => Ok(Py::new(py, PyDateTime { inner: value })?
            .into_bound(py)
            .into_any()),
    }
}
