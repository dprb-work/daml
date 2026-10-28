//! Copy imports must preserve the collection-depth admission boundary.

use daml::{
    AdmissionStage, CopyEdge, NodeSummary, ResolveErrorKind, ResolvedNode, SchemaModule, Schemas,
    StageEdit, StageError, StageOutcome, parse,
};
use serde_json::json;

fn schemas() -> Schemas {
    Schemas::compose([SchemaModule::from_value(json!({"types": {}})).unwrap()]).unwrap()
}

struct CopyAtDepth {
    destination: String,
}

impl AdmissionStage for CopyAtDepth {
    type Error = &'static str;

    fn defer(&mut self, _node: NodeSummary<'_>, path: &str) -> Result<bool, Self::Error> {
        Ok(path == "/target")
    }

    fn materialize(
        &mut self,
        _root: &ResolvedNode<'_>,
        _copies: &[CopyEdge],
        round: usize,
    ) -> Result<StageOutcome<Self::Error>, Self::Error> {
        Ok(if round == 0 {
            StageOutcome::Edits(vec![StageEdit::Copy {
                path: self.destination.clone(),
                from: "/source".into(),
            }])
        } else {
            StageOutcome::Ready { meaning: None }
        })
    }
}

fn nested_input(mapping_depth: usize, source: &str) -> String {
    let mut target = "0".to_owned();
    for _ in 0..mapping_depth {
        target = format!("{{a: {target}}}");
    }
    format!("{{source: {source}\ntarget: {target}}}")
}

fn copied_leaf(root: &ResolvedNode<'_>, nested_maps: usize) -> serde_json::Value {
    let mut node = root.get("target").unwrap();
    for _ in 0..nested_maps {
        node = node.get("a").unwrap();
    }
    node.get("a").unwrap().to_json()
}

#[test]
fn copying_a_collection_onto_a_scalar_at_depth_128_is_rejected() {
    let registry = schemas();
    let document = parse(nested_input(127, "{}")).unwrap();
    let mut stage = CopyAtDepth {
        destination: format!("/target{}", "/a".repeat(127)),
    };

    let StageError::Resolve(error) = registry
        .resolve_staged(&document, None, &[], &mut stage)
        .unwrap_err()
    else {
        panic!("collection imported at depth 128 must be rejected by the copy boundary");
    };
    assert_eq!(error.kind, ResolveErrorKind::LimitExceeded);
}

#[test]
fn scalar_copy_at_depth_128_and_collection_copy_at_depth_127_are_accepted() {
    let registry = schemas();

    let document = parse(nested_input(127, "1")).unwrap();
    let scalar_destination = format!("/target{}", "/a".repeat(127));
    let mut scalar_stage = CopyAtDepth {
        destination: scalar_destination.clone(),
    };
    let resolved = registry
        .resolve_staged(&document, None, &[], &mut scalar_stage)
        .unwrap();
    assert_eq!(copied_leaf(resolved.root(), 126), json!(1));

    let document = parse(nested_input(126, "{}")).unwrap();
    let collection_destination = format!("/target{}", "/a".repeat(126));
    let mut collection_stage = CopyAtDepth {
        destination: collection_destination.clone(),
    };
    let resolved = registry
        .resolve_staged(&document, None, &[], &mut collection_stage)
        .unwrap();
    assert_eq!(copied_leaf(resolved.root(), 125), json!({}));
}
