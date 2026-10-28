//! Pointer admission rejects sequence-index aliases without restricting map keys.

use daml::{
    AdmissionStage, CopyEdge, ErrorOrigin, NodeSummary, ResolveErrorKind, ResolvedNode, Schemas,
    SemanticNode, StageEdit, StageError, StageOutcome, parse,
};
use serde_json::json;

struct Ready;

struct StageCopies {
    from: &'static str,
}

impl AdmissionStage for StageCopies {
    type Error = &'static str;

    fn defer(&mut self, _node: NodeSummary<'_>, _path: &str) -> Result<bool, Self::Error> {
        Ok(false)
    }

    fn materialize(
        &mut self,
        _root: &ResolvedNode<'_>,
        _copies: &[CopyEdge],
        round: usize,
    ) -> Result<StageOutcome<Self::Error>, Self::Error> {
        Ok(if round == 0 {
            StageOutcome::Edits(vec![StageEdit::Copy {
                path: "/target".into(),
                from: self.from.into(),
            }])
        } else {
            StageOutcome::Ready { meaning: None }
        })
    }
}

impl AdmissionStage for Ready {
    type Error = &'static str;

    fn defer(&mut self, _node: NodeSummary<'_>, _path: &str) -> Result<bool, Self::Error> {
        Ok(false)
    }

    fn materialize(
        &mut self,
        _root: &ResolvedNode<'_>,
        _copies: &[CopyEdge],
        _round: usize,
    ) -> Result<StageOutcome<Self::Error>, Self::Error> {
        Ok(StageOutcome::Ready { meaning: None })
    }
}

fn registry() -> Schemas {
    Schemas::compose([]).unwrap()
}

fn replacement(path: &str, value: &str) -> StageEdit {
    StageEdit::Replace {
        path: path.into(),
        value: SemanticNode::from_document(&parse(value).unwrap()),
    }
}

fn assert_invalid(result: Result<daml::StagedResolution<'_>, StageError<&'static str>>) {
    let StageError::Resolve(error) = result.unwrap_err() else {
        panic!("pointer admission must report a structured resolve error");
    };
    assert_eq!(error.kind, ResolveErrorKind::InvalidStage);
    assert!(matches!(error.origin, ErrorOrigin::Input(_)));
}

#[test]
fn initial_replacements_reject_signed_sequence_aliases_before_applying_edits() {
    let registry = registry();
    let document = parse("{items: [{x: 0}]}").unwrap();
    let edits = [
        replacement("/items/0", "1"),
        replacement("/items/+0/x", "2"),
    ];
    let result = registry.resolve_staged(&document, None, &edits, &mut Ready);
    assert_invalid(result);
}

#[test]
fn copy_source_aliases_are_rejected_but_canonical_indices_and_map_keys_work() {
    let registry = registry();
    let document = parse("{items: [{x: 0}]\ntarget: null}").unwrap();
    let result = registry.resolve_staged(
        &document,
        None,
        &[],
        &mut StageCopies {
            from: "/items/+0/x",
        },
    );
    assert_invalid(result);

    let document = parse("{items: [{x: 0}]\nmap: {00: 3\n-0: 4}\ntarget: null}").unwrap();
    let mut stage = StageCopies { from: "/map/00" };
    let resolved = registry
        .resolve_staged(
            &document,
            None,
            &[replacement("/items/0/x", "1")],
            &mut stage,
        )
        .unwrap();
    assert_eq!(
        resolved.root().to_json(),
        json!({
            "items": [{"x": 1}],
            "map": {"00": 3, "-0": 4},
            "target": 3
        })
    );
}
