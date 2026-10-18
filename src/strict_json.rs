use crate::{
    schema::profile::{MAX_RAW_WORK_BYTES, MAX_SCHEMA_DEPTH, MAX_SCHEMA_NODES, MAX_SOURCE_BYTES},
    schema_error::{SchemaError, SchemaErrorKind},
};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value, value::RawValue};
use std::fmt;

const INTERNAL_ERROR: &str = "strict JSON parser stopped";

pub(crate) fn parse(source: &str) -> Result<Value, SchemaError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(limit_error("JSON source exceeds the 8 MiB admission limit"));
    }
    check_container_depth(source)?;

    // RawValue preserves each original JSON subtree without constructing a generic
    // Value first. Parsing each borrowed child separately keeps object dispatch
    // explicit, at the cost of rescanning nested bytes under the work bound below.
    let root: &RawValue = serde_json::from_str(source).map_err(parse_error)?;
    let mut budget = Budget {
        nodes: 0,
        raw_work_bytes: source.len(),
        failure: None,
    };
    let value = parse_node(root, 1, &mut budget)
        .map_err(|error| budget.failure.take().unwrap_or_else(|| parse_error(error)))?;
    Ok(value)
}

struct Budget {
    nodes: usize,
    raw_work_bytes: usize,
    failure: Option<SchemaError>,
}

impl Budget {
    fn charge(&mut self, raw: &RawValue, depth: usize) -> Result<(), serde_json::Error> {
        if depth > MAX_SCHEMA_DEPTH {
            return self.fail(limit_error(
                "JSON nesting exceeds the 128-level admission limit",
            ));
        }
        if self.nodes == MAX_SCHEMA_NODES {
            return self.fail(limit_error(
                "JSON value exceeds the 10,000-node admission limit",
            ));
        }
        let Some(work) = self.raw_work_bytes.checked_add(raw.get().len()) else {
            return self.fail(limit_error(
                "JSON raw-rescan work exceeds the admission limit",
            ));
        };
        if work > MAX_RAW_WORK_BYTES {
            return self.fail(limit_error(
                "JSON raw-rescan work exceeds the 64 MiB admission limit",
            ));
        }
        self.nodes += 1;
        self.raw_work_bytes = work;
        Ok(())
    }

    fn fail<T>(&mut self, error: SchemaError) -> Result<T, serde_json::Error> {
        self.failure = Some(error);
        Err(<serde_json::Error as de::Error>::custom(INTERNAL_ERROR))
    }
}

fn check_container_depth(source: &str) -> Result<(), SchemaError> {
    // This is only a recursion preflight; serde_json still owns JSON syntax.
    // Its RawValue admission has an internal recursion guard with no public
    // typed error kind, so reject excessive container nesting before that pass.
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for byte in source.bytes() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else if byte == b'"' {
            quoted = true;
        } else if matches!(byte, b'{' | b'[') {
            depth = depth.saturating_add(1);
            if depth >= MAX_SCHEMA_DEPTH {
                return Err(limit_error(
                    "JSON nesting exceeds the 128-level admission limit",
                ));
            }
        } else if matches!(byte, b'}' | b']') {
            depth = depth.saturating_sub(1);
        }
    }
    Ok(())
}

fn parse_node(
    raw: &RawValue,
    depth: usize,
    budget: &mut Budget,
) -> Result<Value, serde_json::Error> {
    budget.charge(raw, depth)?;
    let token = raw.get().as_bytes().first().copied();
    let mut deserializer = serde_json::Deserializer::from_str(raw.get());
    let value = match token {
        Some(b'{') => deserializer.deserialize_map(ObjectVisitor { budget, depth })?,
        Some(b'[') => deserializer.deserialize_seq(ArrayVisitor { budget, depth })?,
        _ => Value::deserialize(&mut deserializer)?,
    };
    deserializer.end()?;
    Ok(value)
}

struct ObjectVisitor<'a> {
    budget: &'a mut Budget,
    depth: usize,
}

impl<'de> Visitor<'de> for ObjectVisitor<'_> {
    type Value = Value;

    fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("a JSON object with unique decoded member names")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Value, A::Error> {
        let mut object = Map::new();
        while let Some(key) = entries.next_key::<String>()? {
            if object.contains_key(&key) {
                self.budget.failure = Some(parse_error_message(
                    "duplicate JSON object member after key decoding",
                ));
                return Err(de::Error::custom(INTERNAL_ERROR));
            }
            let child: &'de RawValue = entries.next_value()?;
            let value =
                parse_node(child, self.depth + 1, self.budget).map_err(de::Error::custom)?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

struct ArrayVisitor<'a> {
    budget: &'a mut Budget,
    depth: usize,
}

impl<'de> Visitor<'de> for ArrayVisitor<'_> {
    type Value = Value;

    fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("a JSON array")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut entries: A) -> Result<Value, A::Error> {
        let mut array = Vec::new();
        while let Some(child) = entries.next_element::<&'de RawValue>()? {
            array.push(parse_node(child, self.depth + 1, self.budget).map_err(de::Error::custom)?);
        }
        Ok(Value::Array(array))
    }
}

fn parse_error(error: serde_json::Error) -> SchemaError {
    SchemaError::new(
        SchemaErrorKind::Parse,
        format!("invalid schema JSON: {error}"),
    )
    .caused_by(error)
}

fn parse_error_message(message: &str) -> SchemaError {
    SchemaError::new(SchemaErrorKind::Parse, message)
        .caused_by(<serde_json::Error as de::Error>::custom(message))
}

fn limit_error(message: &str) -> SchemaError {
    SchemaError::new(SchemaErrorKind::LimitExceeded, message)
}
