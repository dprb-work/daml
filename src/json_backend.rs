use crate::resolved::{ResolvedNode, ResolvedValue};
use jsonschema::{
    json::{Array, Json, JsonNumber, Node, NodeIdentity, Object},
    types::JsonType,
};
use serde_json::Value;
use std::{borrow::Cow, collections::BTreeMap, iter::Map};

/// The DAML JSON representation, with tags retained outside JSON data.
pub(crate) struct DamlJson;

/// A cheap handle to a resolved value or a property name validated by `propertyNames`.
#[derive(Clone, Copy)]
pub(crate) enum NodeRef<'a> {
    Value(&'a ResolvedNode<'a>),
    Property(&'a str),
}

impl<'a> NodeRef<'a> {
    /// Creates a validation view of a resolved value.
    pub(crate) fn new(node: &'a ResolvedNode<'a>) -> Self {
        Self::Value(node)
    }

    pub(crate) fn value(self) -> Option<&'a ResolvedValue<'a>> {
        match self {
            Self::Value(node) => Some(&node.value),
            Self::Property(_) => None,
        }
    }
}

pub(crate) fn equal_resolved(
    left: &ResolvedNode<'_>,
    right: &ResolvedNode<'_>,
    tags: bool,
) -> bool {
    if tags
        && left
            .tag
            .as_ref()
            .map(|tag| tag.binding.as_deref().unwrap_or(tag.name.as_ref()))
            != right
                .tag
                .as_ref()
                .map(|tag| tag.binding.as_deref().unwrap_or(tag.name.as_ref()))
    {
        return false;
    }
    match (&left.value, &right.value) {
        (ResolvedValue::Null, ResolvedValue::Null) => true,
        (ResolvedValue::Bool(left), ResolvedValue::Bool(right)) => left == right,
        (ResolvedValue::Number(left), ResolvedValue::Number(right)) => {
            left == right
                || jsonschema::json::cmp::equal_numbers(&NumberRef(left.as_ref()), right.as_ref())
        }
        (ResolvedValue::String(left), ResolvedValue::String(right)) => left == right,
        (ResolvedValue::Temporal(left), ResolvedValue::Temporal(right)) => left == right,
        (ResolvedValue::Sequence(left), ResolvedValue::Sequence(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| equal_resolved(left, right, tags))
        }
        (ResolvedValue::Mapping(left), ResolvedValue::Mapping(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, value)| {
                    right
                        .get(key.as_ref())
                        .is_some_and(|other| equal_resolved(value, other, tags))
                })
        }
        _ => false,
    }
}

impl Json for DamlJson {
    type Node<'a> = NodeRef<'a>;
    type PreparedKey = String;
    type StringBuffer = ();

    fn prepare_key(key: &str) -> Self::PreparedKey {
        key.to_owned()
    }

    fn with_string_node<T>(
        _buffer: &mut Self::StringBuffer,
        string: &str,
        f: impl FnOnce(Self::Node<'_>) -> T,
    ) -> T {
        f(NodeRef::Property(string))
    }
}

pub(crate) struct NumberRef<'a>(pub(crate) &'a serde_json::Number);

impl JsonNumber for NumberRef<'_> {
    fn as_u64(&self) -> Option<u64> {
        self.0.as_u64()
    }

    fn as_i64(&self) -> Option<i64> {
        self.0.as_i64()
    }

    fn as_f64(&self) -> Option<f64> {
        self.0.as_f64()
    }

    fn as_str(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.0.as_str())
    }

    fn to_number(&self) -> Cow<'_, serde_json::Number> {
        Cow::Borrowed(self.0)
    }
}

impl<'a> Node<'a, DamlJson> for NodeRef<'a> {
    type Object = &'a BTreeMap<Cow<'a, str>, ResolvedNode<'a>>;
    type Array = &'a [ResolvedNode<'a>];
    type Number = NumberRef<'a>;

    fn as_object(&self) -> Option<Self::Object> {
        match self.value()? {
            ResolvedValue::Mapping(fields) => Some(fields),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<Self::Array> {
        match self.value()? {
            ResolvedValue::Sequence(items) => Some(items),
            _ => None,
        }
    }

    fn as_string(&self) -> Option<Cow<'a, str>> {
        match self {
            Self::Value(ResolvedNode {
                value: ResolvedValue::String(text),
                ..
            }) => Some(Cow::Borrowed(text.as_ref())),
            Self::Value(ResolvedNode {
                value: ResolvedValue::Temporal(value),
                ..
            }) => Some(Cow::Borrowed(value.as_str())),
            Self::Property(text) => Some(Cow::Borrowed(text)),
            _ => None,
        }
    }

    fn as_number(&self) -> Option<Self::Number> {
        match self.value()? {
            ResolvedValue::Number(number) => Some(NumberRef(number.as_ref())),
            _ => None,
        }
    }

    fn as_boolean(&self) -> Option<bool> {
        match self.value()? {
            ResolvedValue::Bool(value) => Some(*value),
            _ => None,
        }
    }

    fn is_null(&self) -> bool {
        matches!(self.value(), Some(ResolvedValue::Null))
    }

    fn is_number(&self) -> bool {
        matches!(self.value(), Some(ResolvedValue::Number(_)))
    }

    fn json_type(&self) -> JsonType {
        match self {
            Self::Property(_) => JsonType::String,
            Self::Value(node) => match node.value {
                ResolvedValue::Null => JsonType::Null,
                ResolvedValue::Bool(_) => JsonType::Boolean,
                ResolvedValue::Number(_) => JsonType::Number,
                ResolvedValue::String(_) | ResolvedValue::Temporal(_) => JsonType::String,
                ResolvedValue::Sequence(_) => JsonType::Array,
                ResolvedValue::Mapping(_) => JsonType::Object,
            },
        }
    }

    fn equals_value(&self, expected: &Value) -> bool {
        match (self, expected) {
            (Self::Property(name), Value::String(expected)) => *name == expected,
            (Self::Value(node), expected) => match (&node.value, expected) {
                (ResolvedValue::Null, Value::Null) => true,
                (ResolvedValue::Bool(value), Value::Bool(expected)) => value == expected,
                (ResolvedValue::Number(value), Value::Number(expected)) => {
                    jsonschema::json::cmp::equal_numbers(&NumberRef(value.as_ref()), expected)
                }
                (ResolvedValue::String(value), Value::String(expected)) => value == expected,
                (ResolvedValue::Temporal(value), Value::String(expected)) => {
                    value.as_str() == expected
                }
                (ResolvedValue::Sequence(items), Value::Array(expected)) => {
                    items.len() == expected.len()
                        && items
                            .iter()
                            .zip(expected)
                            .all(|(item, expected)| NodeRef::new(item).equals_value(expected))
                }
                (ResolvedValue::Mapping(fields), Value::Object(expected)) => {
                    fields.len() == expected.len()
                        && fields.iter().all(|(key, value)| {
                            expected
                                .get(key.as_ref())
                                .is_some_and(|expected| NodeRef::new(value).equals_value(expected))
                        })
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn to_value(&self) -> Cow<'a, Value> {
        match self {
            Self::Value(node) => Cow::Owned(node.to_json()),
            Self::Property(name) => Cow::Owned(Value::String((*name).to_owned())),
        }
    }

    // Schema cycles are rejected and materialized values are depth-bounded. Omitting identity
    // avoids assigning a false identity to a cheap copied view; the engine's cycle paths remain
    // bounded by those composition and resolution limits.
    fn identity(&self) -> Option<NodeIdentity> {
        None
    }
}

impl<'a> Object<'a, DamlJson> for &'a BTreeMap<Cow<'a, str>, ResolvedNode<'a>> {
    type Node = NodeRef<'a>;
    type MemberName = &'a str;
    type MembersIter = Map<
        std::collections::btree_map::Iter<'a, Cow<'a, str>, ResolvedNode<'a>>,
        fn((&'a Cow<'a, str>, &'a ResolvedNode<'a>)) -> (&'a str, NodeRef<'a>),
    >;

    fn len(&self) -> usize {
        BTreeMap::len(self)
    }

    fn get(&self, key: &String) -> Option<Self::Node> {
        BTreeMap::get(self, key.as_str()).map(NodeRef::new)
    }

    fn members(&self) -> Self::MembersIter {
        fn member<'a>(
            (key, value): (&'a Cow<'a, str>, &'a ResolvedNode<'a>),
        ) -> (&'a str, NodeRef<'a>) {
            (key.as_ref(), NodeRef::new(value))
        }
        BTreeMap::iter(self).map(member)
    }
}

impl<'a> Array<'a, DamlJson> for &'a [ResolvedNode<'a>] {
    type Node = NodeRef<'a>;
    type ElementsIter =
        Map<std::slice::Iter<'a, ResolvedNode<'a>>, fn(&'a ResolvedNode<'a>) -> NodeRef<'a>>;

    fn len(&self) -> usize {
        <[ResolvedNode<'a>]>::len(self)
    }

    fn elements(&self) -> Self::ElementsIter {
        fn node<'a>(value: &'a ResolvedNode<'a>) -> NodeRef<'a> {
            NodeRef::new(value)
        }
        self.iter().map(node)
    }

    fn is_unique(&self) -> bool {
        self.iter().enumerate().all(|(index, value)| {
            !self[index + 1..]
                .iter()
                .any(|other| equal_resolved(value, other, false))
        })
    }
}
