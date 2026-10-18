use serde_json::Value;

pub(crate) const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_SCHEMA_NODES: usize = 10_000;
pub(crate) const MAX_SCHEMA_DEPTH: usize = 128;
pub(crate) const MAX_COMPILE_NODES: usize = 100_000;
pub(crate) const MAX_RAW_WORK_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy)]
enum Shape {
    Map,
    Array,
    Single,
}

struct Classification {
    shape: Shape,
    applies: bool,
    unsupported_defaults: bool,
}

// Schema positions and their evaluation/default roles have one owner. In
// particular, definition containment is not an applicable constraint, while
// schema-looking values inside annotations are never schema children.
fn classify(keyword: &str, value: &Value, pattern_properties: bool) -> Option<Classification> {
    let (shape, applies, unsupported_defaults) = match keyword {
        "properties" => (Shape::Map, true, false),
        "patternProperties" | "dependentSchemas" => (Shape::Map, true, true),
        "$defs" | "definitions" => (Shape::Map, false, false),
        "allOf" | "anyOf" | "oneOf" | "prefixItems" => (Shape::Array, true, false),
        "items" => (Shape::Single, true, false),
        "additionalProperties" => (Shape::Single, true, pattern_properties),
        "not"
        | "contains"
        | "propertyNames"
        | "if"
        | "then"
        | "else"
        | "unevaluatedProperties"
        | "unevaluatedItems"
        | "additionalItems"
        | "contentSchema" => (Shape::Single, true, true),
        _ => return None,
    };
    let valid_shape = match shape {
        Shape::Map => value.is_object(),
        Shape::Array => value.is_array(),
        Shape::Single => value.is_object() || value.is_boolean(),
    };
    valid_shape.then_some(Classification {
        shape,
        applies,
        unsupported_defaults,
    })
}

enum Segment<'a> {
    Key(&'a str),
    Index(usize),
}

pub(super) struct Child<'a> {
    keyword: &'a str,
    segment: Option<Segment<'a>>,
    pub value: &'a Value,
    pub applies: bool,
    pub unsupported_defaults: bool,
}

impl Child<'_> {
    pub(super) fn pointer(&self, parent: &str) -> String {
        let keyword = crate::pointer::join(parent, self.keyword);
        match &self.segment {
            Some(Segment::Key(key)) => crate::pointer::join(&keyword, key),
            Some(Segment::Index(index)) => crate::pointer::join(&keyword, &index.to_string()),
            None => keyword,
        }
    }
}

enum Nested<'a> {
    Empty,
    Map(serde_json::map::Iter<'a>),
    Array(std::iter::Enumerate<std::slice::Iter<'a, Value>>),
    Single(Option<&'a Value>),
}

impl<'a> Nested<'a> {
    fn next(&mut self) -> Option<(Option<Segment<'a>>, &'a Value)> {
        match self {
            Self::Empty => None,
            Self::Map(values) => values
                .next()
                .map(|(key, value)| (Some(Segment::Key(key)), value)),
            Self::Array(values) => values
                .next()
                .map(|(index, value)| (Some(Segment::Index(index)), value)),
            Self::Single(value) => value.take().map(|value| (None, value)),
        }
    }
}

pub(super) struct Children<'a> {
    fields: Option<serde_json::map::Iter<'a>>,
    nested: Nested<'a>,
    keyword: &'a str,
    applies: bool,
    unsupported_defaults: bool,
    pattern_properties: bool,
}

impl<'a> Iterator for Children<'a> {
    type Item = Child<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((segment, value)) = self.nested.next() {
                return Some(Child {
                    keyword: self.keyword,
                    segment,
                    value,
                    applies: self.applies,
                    unsupported_defaults: self.unsupported_defaults,
                });
            }
            let (keyword, value) = self.fields.as_mut()?.next()?;
            let Some(kind) = classify(keyword, value, self.pattern_properties) else {
                continue;
            };
            self.keyword = keyword;
            self.applies = kind.applies;
            self.unsupported_defaults = kind.unsupported_defaults;
            self.nested = match kind.shape {
                Shape::Map => Nested::Map(value.as_object().expect("classified object").iter()),
                Shape::Array => Nested::Array(
                    value
                        .as_array()
                        .expect("classified array")
                        .iter()
                        .enumerate(),
                ),
                Shape::Single => Nested::Single(Some(value)),
            };
        }
    }
}

pub(super) fn children(value: &Value) -> Children<'_> {
    Children {
        fields: value.as_object().map(|map| map.iter()),
        nested: Nested::Empty,
        keyword: "",
        applies: true,
        unsupported_defaults: false,
        pattern_properties: value.get("patternProperties").is_some(),
    }
}

pub(super) fn try_for_each_child_mut<E>(
    value: &mut Value,
    mut visit: impl FnMut(&mut Value) -> Result<(), E>,
) -> Result<(), E> {
    let Some(map) = value.as_object_mut() else {
        return Ok(());
    };
    let patterns = map.contains_key("patternProperties");
    for (keyword, value) in map {
        let Some(kind) = classify(keyword, value, patterns) else {
            continue;
        };
        match kind.shape {
            Shape::Map => {
                for child in value
                    .as_object_mut()
                    .expect("classified object")
                    .values_mut()
                {
                    visit(child)?;
                }
            }
            Shape::Array => {
                for child in value.as_array_mut().expect("classified array") {
                    visit(child)?;
                }
            }
            Shape::Single => visit(value)?,
        }
    }
    Ok(())
}
