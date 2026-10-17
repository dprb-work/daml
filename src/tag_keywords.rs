use crate::json_backend::{DamlJson, NodeRef};
use jsonschema::{Keyword, ValidationError};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Immutable type-to-contract memberships used by `x-daml-slot` validators.
pub(crate) struct TagRules {
    pub(crate) memberships: HashMap<String, HashSet<String>>,
}

/// Requires an explicit node tag from a set of literal names.
pub(crate) struct TagKeyword {
    allowed: HashSet<String>,
}

impl TagKeyword {
    /// Parses the supported `x-daml-tag` string or nonempty unique string array.
    pub(crate) fn new(value: &Value) -> Result<Self, String> {
        let mut allowed = HashSet::new();
        match value {
            Value::String(name) => insert_tag(&mut allowed, name)?,
            Value::Array(names) if !names.is_empty() => {
                for value in names {
                    let Value::String(name) = value else {
                        return Err("x-daml-tag entries must be strings".to_owned());
                    };
                    insert_tag(&mut allowed, name)?;
                }
            }
            Value::Array(_) => return Err("x-daml-tag array must not be empty".to_owned()),
            _ => return Err("x-daml-tag must be a string or nonempty array of strings".to_owned()),
        }
        Ok(Self { allowed })
    }

    fn is_valid_node(&self, node: NodeRef<'_>) -> bool {
        node.tag().is_some_and(|tag| self.allowed.contains(tag))
    }
}

fn insert_tag(allowed: &mut HashSet<String>, name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("x-daml-tag names must not be empty".to_owned());
    }
    if !allowed.insert(name.to_owned()) {
        return Err(format!("x-daml-tag contains duplicate name {name:?}"));
    }
    Ok(())
}

impl<'i> Keyword<'i, DamlJson> for TagKeyword {
    fn validate(&self, instance: NodeRef<'i>) -> Result<(), ValidationError<'i>> {
        if self.is_valid_node(instance) {
            Ok(())
        } else {
            Err(ValidationError::custom(
                "DAML tag does not match x-daml-tag",
            ))
        }
    }

    fn is_valid(&self, instance: NodeRef<'i>) -> bool {
        self.is_valid_node(instance)
    }
}

/// Requires an explicit tag whose compiled binding implements one contract.
pub(crate) struct SlotKeyword {
    contract: String,
    rules: Arc<TagRules>,
}

impl SlotKeyword {
    /// Parses the literal contract name and shares immutable composed memberships.
    pub(crate) fn new(value: &Value, rules: Arc<TagRules>) -> Result<Self, String> {
        let Value::String(contract) = value else {
            return Err("x-daml-slot must be a contract-name string".to_owned());
        };
        if contract.is_empty() {
            return Err("x-daml-slot contract name must not be empty".to_owned());
        }
        Ok(Self {
            contract: contract.clone(),
            rules,
        })
    }

    fn is_valid_node(&self, node: NodeRef<'_>) -> bool {
        node.tag().is_some_and(|tag| {
            self.rules
                .memberships
                .get(tag)
                .is_some_and(|contracts| contracts.contains(&self.contract))
        })
    }
}

impl<'i> Keyword<'i, DamlJson> for SlotKeyword {
    fn validate(&self, instance: NodeRef<'i>) -> Result<(), ValidationError<'i>> {
        if self.is_valid_node(instance) {
            Ok(())
        } else {
            Err(ValidationError::custom(
                "DAML tag does not satisfy x-daml-slot",
            ))
        }
    }

    fn is_valid(&self, instance: NodeRef<'i>) -> bool {
        self.is_valid_node(instance)
    }
}
