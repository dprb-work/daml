use crate::{ResolutionLimits, ResolveError, ResolvedDocument, ResolvedNode, SchemaLocation};
use serde::ser::{self, Impossible, Serialize, SerializeSeq, SerializeStruct};
use std::{error::Error, fmt};

/// A checked contract applying at the selected value or one of its ancestors.
#[derive(Debug)]
pub struct ExplanationConstraint<'a> {
    /// RFC 6901 instance pointer where this contract applies.
    pub instance_path: String,
    /// Canonical schema resource and pointer, without expanding references.
    pub location: &'a SchemaLocation,
    /// The admitted, registry-owned schema, including its original references.
    pub schema: &'a serde_json::Value,
}

/// A selected value retaining its fully admitted document and applicable contracts.
#[derive(Debug)]
pub struct Explanation<'a> {
    pub(crate) document: ResolvedDocument<'a>,
    pub(crate) path: String,
    pub(crate) constraints: Vec<ExplanationConstraint<'a>>,
}

impl<'a> Explanation<'a> {
    /// Returns the selected RFC 6901 instance pointer; empty denotes the root.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the checked value, retaining exact scalar kinds, tags and origins.
    pub fn node(&self) -> &ResolvedNode<'a> {
        self.document
            .root()
            .at_pointer(&self.path)
            .expect("the explanation pointer was admitted against this immutable document")
    }

    /// Returns direct and enclosing contracts, each at its actual instance path.
    pub fn constraints(&self) -> &[ExplanationConstraint<'a>] {
        &self.constraints
    }
}

/// Failure to admit a document or select a value for explanation.
#[derive(Debug)]
pub enum ExplainError {
    /// Normal completion, validation, numeric or cumulative resource failure.
    Resolve(ResolveError),
    /// Neither a root assertion nor an expected root contract was supplied.
    MissingRootContract,
    /// The requested pointer has invalid RFC 6901 syntax or an invalid array index.
    InvalidPointer(String),
    /// A well-formed pointer does not select a value in the completed document.
    PathNotFound(String),
}

impl fmt::Display for ExplainError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolve(error) => fmt::Display::fmt(error, out),
            Self::MissingRootContract => {
                out.write_str("explanation requires a root assertion or an expected type")
            }
            Self::InvalidPointer(path) => write!(out, "invalid RFC 6901 instance pointer {path:?}"),
            Self::PathNotFound(path) => {
                write!(out, "no resolved value at instance pointer {path:?}")
            }
        }
    }
}

impl Error for ExplainError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolve(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ResolveError> for ExplainError {
    fn from(error: ResolveError) -> Self {
        Self::Resolve(error)
    }
}

pub(crate) fn select_pointer<'n, 'a>(
    root: &'n ResolvedNode<'a>,
    pointer: &str,
) -> Result<&'n ResolvedNode<'a>, ExplainError> {
    if pointer.is_empty() {
        return Ok(root);
    }
    let invalid = || ExplainError::InvalidPointer(pointer.to_owned());
    let missing = || ExplainError::PathNotFound(pointer.to_owned());
    let parts = pointer.strip_prefix('/').ok_or_else(invalid)?;
    // Check all escapes, even if traversal would stop at an absent earlier key.
    let mut bytes = pointer.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'~' && !matches!(bytes.next(), Some(b'0' | b'1')) {
            return Err(invalid());
        }
    }
    let mut node = root;
    for part in parts.split('/') {
        let key = crate::pointer::decode_segment(part).ok_or_else(invalid)?;
        node = match node.value() {
            crate::ResolvedValue::Mapping(fields) => {
                fields.get(key.as_ref()).ok_or_else(missing)?
            }
            crate::ResolvedValue::Sequence(items) => {
                if key.is_empty()
                    || (key.len() > 1 && key.starts_with('0'))
                    || !key.bytes().all(|byte| byte.is_ascii_digit())
                {
                    return Err(invalid());
                }
                let index = key.parse::<usize>().map_err(|_| invalid())?;
                items.get(index).ok_or_else(missing)?
            }
            _ => return Err(missing()),
        };
    }
    Ok(node)
}

pub(crate) struct EvaluationUnit {
    pub(crate) schema_location: String,
    pub(crate) instance_location: String,
}

#[derive(Debug)]
pub(crate) enum CaptureError {
    Limit,
    Invalid(String),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit => {
                out.write_str("explanation metadata exceeds remaining resolution budget")
            }
            Self::Invalid(message) => out.write_str(message),
        }
    }
}

impl Error for CaptureError {}

impl ser::Error for CaptureError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self::Invalid(message.to_string())
    }
}

pub(crate) fn reserve(
    limits: &mut ResolutionLimits,
    nodes: usize,
    bytes: usize,
    steps: usize,
) -> Result<(), CaptureError> {
    if nodes > limits.max_nodes || bytes > limits.max_bytes || steps > limits.max_steps {
        return Err(CaptureError::Limit);
    }
    limits.max_nodes -= nodes;
    limits.max_bytes -= bytes;
    limits.max_steps -= steps;
    Ok(())
}

/// The dependency exposes evaluation units through Serialize, but not a public
/// node iterator. Capture only their metadata directly: no JSON buffer, reparse,
/// annotations, error rendering, or independent branch evaluator is needed.
pub(crate) fn capture_evaluation(
    evaluation: &jsonschema::Evaluation,
    remaining: &mut ResolutionLimits,
    emit: &mut dyn FnMut(EvaluationUnit, &mut ResolutionLimits) -> Result<bool, CaptureError>,
) -> Result<(), CaptureError> {
    let mut capture = Capture { remaining, emit };
    evaluation
        .hierarchical()
        .serialize(CaptureSerializer(&mut capture))?;
    Ok(())
}

struct Capture<'a> {
    remaining: &'a mut ResolutionLimits,
    // False suppresses this unit's descendants, for example property-name
    // schemas which constrain keys rather than the containing object's value.
    emit: &'a mut dyn FnMut(EvaluationUnit, &mut ResolutionLimits) -> Result<bool, CaptureError>,
}

// Only these scalar fields and the hierarchy's structs/sequences are consumed.
// Unknown output fields are deliberately not serialized at all.
enum Captured {
    Unit,
    Bool(bool),
    Text(String),
}

struct CaptureSerializer<'a, 'c>(&'a mut Capture<'c>);
struct UnitCapture<'a, 'c> {
    capture: &'a mut Capture<'c>,
    valid: Option<bool>,
    schema: Option<String>,
    instance: Option<String>,
    descend: Option<bool>,
}
struct SequenceCapture<'a, 'c>(&'a mut Capture<'c>);

// Serde requires implementations for data kinds the evaluator never emits at
// this seam. Reject them instead of silently accepting an upstream shape change.
macro_rules! unsupported_scalar {
    ($($method:ident($ty:ty)),* $(,)?) => {
        $(fn $method(self, _value: $ty) -> Result<Self::Ok, Self::Error> {
            Err(CaptureError::Invalid("unexpected evaluation metadata type".into()))
        })*
    };
}

impl<'a, 'c> ser::Serializer for CaptureSerializer<'a, 'c> {
    type Ok = Captured;
    type Error = CaptureError;
    type SerializeSeq = SequenceCapture<'a, 'c>;
    type SerializeTuple = Impossible<Captured, CaptureError>;
    type SerializeTupleStruct = Impossible<Captured, CaptureError>;
    type SerializeTupleVariant = Impossible<Captured, CaptureError>;
    type SerializeMap = Impossible<Captured, CaptureError>;
    type SerializeStruct = UnitCapture<'a, 'c>;
    type SerializeStructVariant = Impossible<Captured, CaptureError>;

    fn serialize_bool(self, value: bool) -> Result<Self::Ok, Self::Error> {
        Ok(Captured::Bool(value))
    }

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        reserve(self.0.remaining, 0, value.len(), value.len().div_ceil(64))?;
        Ok(Captured::Text(value.to_owned()))
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        reserve(self.0.remaining, 1, 0, 1)?;
        Ok(UnitCapture {
            capture: self.0,
            valid: None,
            schema: None,
            instance: None,
            descend: None,
        })
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(SequenceCapture(self.0))
    }

    unsupported_scalar! {
        serialize_i8(i8), serialize_i16(i16), serialize_i32(i32), serialize_i64(i64),
        serialize_u8(u8), serialize_u16(u16), serialize_u32(u32), serialize_u64(u64),
        serialize_f32(f32), serialize_f64(f64), serialize_char(char), serialize_bytes(&[u8]),
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Err(CaptureError::Invalid(
            "unexpected absent evaluation metadata".into(),
        ))
    }
    fn serialize_some<T: Serialize + ?Sized>(self, _value: &T) -> Result<Self::Ok, Self::Error> {
        Err(CaptureError::Invalid(
            "unexpected optional evaluation metadata".into(),
        ))
    }
    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Err(CaptureError::Invalid(
            "unexpected unit evaluation metadata".into(),
        ))
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.serialize_unit()
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.serialize_unit()
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.serialize_unit()
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.serialize_unit()
    }
    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(CaptureError::Invalid("unexpected evaluation tuple".into()))
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(CaptureError::Invalid("unexpected evaluation tuple".into()))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(CaptureError::Invalid("unexpected evaluation tuple".into()))
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(CaptureError::Invalid("unexpected evaluation map".into()))
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(CaptureError::Invalid(
            "unexpected evaluation variant".into(),
        ))
    }
}

impl UnitCapture<'_, '_> {
    fn emit(&mut self) -> Result<bool, CaptureError> {
        if let Some(descend) = self.descend {
            return Ok(descend);
        }
        let valid = self
            .valid
            .ok_or_else(|| CaptureError::Invalid("evaluation unit has no validity flag".into()))?;
        let descend = if valid {
            let unit = EvaluationUnit {
                schema_location: self.schema.take().ok_or_else(|| {
                    CaptureError::Invalid("evaluation unit has no schema location".into())
                })?,
                instance_location: self.instance.take().ok_or_else(|| {
                    CaptureError::Invalid("evaluation unit has no instance location".into())
                })?,
            };
            (self.capture.emit)(unit, self.capture.remaining)?
        } else {
            false
        };
        self.descend = Some(descend);
        Ok(descend)
    }
}

impl SerializeStruct for UnitCapture<'_, '_> {
    type Ok = Captured;
    type Error = CaptureError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        match key {
            "valid" => match value.serialize(CaptureSerializer(self.capture))? {
                Captured::Bool(valid) => self.valid = Some(valid),
                _ => {
                    return Err(CaptureError::Invalid(
                        "invalid evaluation validity flag".into(),
                    ));
                }
            },
            "schemaLocation" | "instanceLocation" if self.valid != Some(false) => {
                let Captured::Text(text) = value.serialize(CaptureSerializer(self.capture))? else {
                    return Err(CaptureError::Invalid("invalid evaluation location".into()));
                };
                if key == "schemaLocation" {
                    self.schema = Some(text);
                } else {
                    self.instance = Some(text);
                }
            }
            "details" if self.emit()? => {
                value.serialize(CaptureSerializer(self.capture))?;
            }
            _ => {}
        }
        Ok(())
    }

    fn end(mut self) -> Result<Self::Ok, Self::Error> {
        self.emit()?;
        Ok(Captured::Unit)
    }
}

impl SerializeSeq for SequenceCapture<'_, '_> {
    type Ok = Captured;
    type Error = CaptureError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        match value.serialize(CaptureSerializer(self.0))? {
            Captured::Unit => Ok(()),
            _ => Err(CaptureError::Invalid(
                "invalid evaluation child unit".into(),
            )),
        }
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(Captured::Unit)
    }
}
