//! Representative native consumer conversion, without importing the consumer runtime.

use daml::{Bundle, NativeErrorKind, Origin, ResolveErrorKind, parse};
use serde_json::json;
use std::path::Path;

fn bundle() -> Bundle {
    Bundle::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/consumer")).unwrap()
}

#[test]
fn recipe_completes_across_private_bundles_and_retains_nested_assertions() {
    let input = parse(include_str!("fixtures/consumer/data/recipe.daml")).unwrap();
    let schemas = bundle().compile(&input, None).unwrap();
    let resolved = schemas.resolve(&input).unwrap();
    assert_eq!(
        resolved.to_json(),
        json!({
            "setup_id": "daily",
            "mode": "task",
            "params": {},
            "adapters": {"warehouse": {}, "cache": {}},
            "execution": {"actor_error_mode": "fail_fast"},
            "optimize": {
                "objective": "score", "walker": "search",
                "budget": {"candidates": 4, "concurrency": 1}
            },
            "steps": [{
                "query": "select value\nfrom metrics", "quest": {},
                "filter_timestamp": true, "unpack": ["value", 0]
            }]
        })
    );
    let root = resolved.root();
    assert_eq!(root.tag(), Some("olaf.config.Recipe"));
    let adapters = root.get("adapters").unwrap();
    assert_eq!(
        adapters.get("warehouse").unwrap().tag(),
        Some("questdb.adapter")
    );
    assert_eq!(adapters.get("cache").unwrap().tag(), Some("redis.adapter"));
    let step = &root.get("steps").unwrap().items().unwrap()[0];
    assert_eq!(step.tag(), Some("questdb.json_query"));
    assert_eq!(step.get("quest").unwrap().tag(), Some("questdb.adapter"));
    assert!(matches!(
        root.get("mode").unwrap().origin(),
        Origin::Default(_)
    ));
}

#[test]
fn representative_constraints_reject_invalid_values_at_their_data_paths() {
    for (source, path) in [
        (
            "!olaf.config.Recipe { setup_id: \"daily\"\n mode: \"thread\" }",
            "/mode",
        ),
        (
            "!olaf.config.OptimizeBudget { candidates: \"many\" }",
            "/candidates",
        ),
        ("!questdb.json_query { query: 9 }", "/query"),
        (
            "!questdb.json_query { query: \"select\"\n unpack: [true] }",
            "/unpack",
        ),
        (
            "!olaf.config.Recipe { setup_id: \"daily\"\n adapters: { warehouse: 1 } }",
            "/adapters/warehouse",
        ),
    ] {
        let input = parse(source).unwrap();
        let schemas = bundle().compile(&input, None).unwrap();
        let error = schemas.resolve(&input).unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::SchemaViolation, "{source}");
        assert_eq!(error.path, path, "{source}");
    }
}

#[test]
fn protocols_are_structural_but_bundle_aliases_remain_private() {
    let input =
        parse("!olaf.config.Recipe { setup_id: \"daily\"\n adapters: { warehouse: {} } }").unwrap();
    let schemas = bundle().compile(&input, None).unwrap();
    assert_eq!(
        schemas.resolve(&input).unwrap().to_json(),
        json!({
            "setup_id": "daily", "mode": "task", "params": {},
            "optimize": null, "adapters": {"warehouse": {}}
        })
    );
    let private = parse("!api.Adapter").unwrap();
    let error = bundle().compile(&private, None).err().unwrap();
    assert_eq!(error.kind, NativeErrorKind::UnknownReference);
    assert_eq!(error.reference.as_deref(), Some("api.Adapter"));
}
