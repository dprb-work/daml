use daml::{
    Bundle, FormatOptions, Origin, SchemaModule, Schemas, SemanticNode, SemanticValue, parse,
};
use serde_json::{Value, json};
use std::{env, fs, io};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture_path = env::args_os()
        .nth(1)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "fixture path is required"))?;
    let fixture: Value = serde_json::from_slice(&fs::read(fixture_path)?)?;
    let root = fixture.get("root").and_then(Value::as_str).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "fixture.root must be a string")
    })?;
    let sources = fixture
        .get("sources")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "fixture.sources must be an object",
            )
        })?
        .iter()
        .map(|(path, source)| {
            source
                .as_str()
                .map(|source| (path.clone(), source.to_owned()))
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "source values must be strings")
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let source = fixture
        .get("source")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "fixture.source must be a string",
            )
        })?;
    let expected = fixture.get("expected").and_then(Value::as_str);
    let bundle = Bundle::from_sources(root, sources)?;
    let document = parse(source)?;
    let schemas = bundle.compile(&document, expected)?;
    let resolved = match expected {
        Some(expected) => schemas.resolve_as(&document, expected)?,
        None => schemas.resolve(&document)?,
    };
    let default_node = resolved.root().get("retries").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "fixture default retries value is missing",
        )
    })?;
    let default_origin = match default_node.origin() {
        Origin::Default(location) => json!({
            "resource": location.resource,
            "pointer": location.pointer,
        }),
        Origin::Input(_) | Origin::Semantic => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "fixture retries value must come from a schema default",
            )
            .into());
        }
    };
    let semantic = SemanticNode::from_resolved(resolved.root());
    let canonical_source = schemas.format_semantic(&semantic, FormatOptions::default())?;

    let tagless_canonical_source =
        if let Some(source) = fixture.get("taglessSource").and_then(Value::as_str) {
            let expected = expected.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "taglessSource requires fixture.expected",
                )
            })?;
            let tagless_document = parse(source)?;
            let tagless_resolved = schemas.resolve_as(&tagless_document, expected)?;
            let tagless_semantic = SemanticNode::from_resolved(tagless_resolved.root());
            Some(schemas.format_semantic(&tagless_semantic, FormatOptions::default())?)
        } else {
            None
        };
    let rebase_source = fixture
        .get("rebaseSource")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "fixture.rebaseSource must be a string",
            )
        })?;
    let rebase_expected = fixture
        .get("rebaseExpected")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "fixture.rebaseExpected must be a string",
            )
        })?;
    let rebase_document = parse(rebase_source)?;
    let rebase_schemas = bundle.compile(&rebase_document, Some(rebase_expected))?;
    let rebase_resolved = rebase_schemas.resolve_as(&rebase_document, rebase_expected)?;
    let rebase_semantic = SemanticNode::from_resolved(rebase_resolved.root());
    let rebase_canonical_source =
        rebase_schemas.format_semantic(&rebase_semantic, FormatOptions::default())?;

    let re_admitted_document = parse(&rebase_canonical_source)?;
    let re_admitted_schemas = bundle.compile(&re_admitted_document, Some(rebase_expected))?;
    let re_admitted_resolved =
        re_admitted_schemas.resolve_as(&re_admitted_document, rebase_expected)?;
    let re_admitted_semantic = SemanticNode::from_resolved(re_admitted_resolved.root());
    let daml::SemanticValue::Mapping(before_entries) = &rebase_semantic.value else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rebase fixture root must be a mapping",
        )
        .into());
    };
    let daml::SemanticValue::Mapping(after_entries) = &re_admitted_semantic.value else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "re-admitted root must be a mapping",
        )
        .into());
    };
    let before_seed = before_entries
        .get("seed")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "rebase seed is missing"))?;
    let after_seed = after_entries
        .get("seed")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "re-admitted seed is missing"))?;
    let before_tag = before_seed.tag.as_ref().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "resolved seed tag is missing")
    })?;
    let after_tag = after_seed.tag.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "re-admitted seed tag is missing",
        )
    })?;
    if before_tag.name != "self.Base"
        || before_tag.binding.is_none()
        || after_tag.name != "app.Base"
        || before_seed.value != SemanticValue::String("seed".into())
        || after_seed.value != SemanticValue::String("seed".into())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "qualified writer must emit app.Base and preserve the default seed value",
        )
        .into());
    }

    let module_sources = fixture
        .get("schemaModuleSources")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "schemaModuleSources is required",
            )
        })?;
    let modules = module_sources
        .iter()
        .map(|source| -> Result<_, Box<dyn std::error::Error>> {
            let source = source.as_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "module source must be a string")
            })?;
            Ok(SchemaModule::from_json(source)?)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let module_schemas = Schemas::compose(modules)?;
    let module_source = fixture
        .get("schemaModuleSource")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "schemaModuleSource is required")
        })?;
    let module_expected = fixture
        .get("schemaModuleExpected")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "schemaModuleExpected is required",
            )
        })?;
    let module_document = parse(module_source)?;
    let module_resolution = module_schemas.resolve_as(&module_document, module_expected)?;
    let module_semantic = SemanticNode::from_resolved(module_resolution.root());
    let module_canonical_source =
        module_schemas.format_semantic(&module_semantic, FormatOptions::default())?;
    let module_default = module_resolution.root().get("attempts").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "module default attempts is missing",
        )
    })?;
    let module_default_origin = match module_default.origin() {
        Origin::Default(location) => json!({
            "resource": location.resource,
            "pointer": location.pointer,
        }),
        Origin::Input(_) | Origin::Semantic => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "module attempts must originate from the registered resource default",
            )
            .into());
        }
    };

    let recursive_field = |name: &str| {
        fixture.get(name).and_then(Value::as_str).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, format!("{name} is required"))
        })
    };
    let recursive_schemas = Schemas::compose([SchemaModule::from_json(recursive_field(
        "recursiveModule",
    )?)?])?;
    let recursive_document = parse(recursive_field("recursiveSource")?)?;
    let recursive_resolution =
        recursive_schemas.resolve_as(&recursive_document, recursive_field("recursiveExpected")?)?;
    let recursive_value = SemanticNode::from_resolved(recursive_resolution.root());
    recursive_schemas.validate_semantic(
        &recursive_value,
        Some(recursive_field("recursiveExpected")?),
    )?;
    let recursive_canonical =
        recursive_schemas.format_semantic(&recursive_value, FormatOptions::default())?;
    let recursive_origin = recursive_resolution
        .root()
        .get("next")
        .unwrap()
        .get("copies")
        .unwrap()
        .origin();
    let Origin::Default(recursive_location) = recursive_origin else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recursive default must retain its schema origin",
        )
        .into());
    };

    println!(
        "{}",
        json!({
            "canonicalSource": canonical_source,
            "taglessCanonicalSource": tagless_canonical_source,
            "rebaseCanonicalSource": rebase_canonical_source,
            "defaultOrigin": default_origin,
            "schemaModuleCanonicalSource": module_canonical_source,
            "schemaModuleDefaultOrigin": module_default_origin,
            "recursiveCanonicalSource": recursive_canonical,
            "recursiveDefaultOrigin": {
                "resource": recursive_location.resource,
                "pointer": recursive_location.pointer,
            }
        })
    );
    Ok(())
}
