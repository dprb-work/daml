//! Adapt a consumer-owned schema format to DAML's schema interchange.
//!
//! This fixture uses a made-up vendor format. Its `kind` and `default_tag`
//! fields belong to the consumer; DAML receives JSON Schema constraints,
//! resources, and explicit assertions on default-generated values.

use daml::{SchemaModule, Schemas};
use serde_json::{Value, json};
use std::{
    error::Error,
    io::{self, Write},
    process::ExitCode,
};

const RESOURCE: &str = "https://schemas.example.invalid/briefing/item";
const SHORT_TAG: &str = "vendor.briefing.Item";
const CANONICAL_TAG: &str = "vendor.briefing.BriefingItem";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    // Record markers and default assertions are translated here, outside
    // DAML's schema loader and resolver. Names are bound separately below.
    let vendor_schema: Value = serde_json::from_str(
        r#"{
          "kind":"record",
          "properties":{
            "name":{"type":"string","minLength":1},
            "goal":{"type":"string","minLength":1},
            "settings":{
              "kind":"record",
              "default":{"active":true,"retries":2},
              "default_tag":"vendor.briefing.Settings",
              "properties":{"active":{"type":"boolean"},"retries":{"type":"integer"}},
              "required":["active","retries"],
              "additionalProperties":false
            }
          },
          "required":["name","goal","settings"],
          "additionalProperties":false
        }"#,
    )?;
    let native_schema = adapt_vendor_schema(vendor_schema)?;

    // Both external names are regular native bindings to one canonical
    // resource. DAML does not parse aliases or normalize vendor references.
    let module = SchemaModule::from_value(json!({
        "types": {
            (SHORT_TAG): {"schema": {"$ref": RESOURCE}},
            (CANONICAL_TAG): {"schema": {"$ref": RESOURCE}},
            "vendor.briefing.Settings": {"schema": {
                "$ref": format!("{RESOURCE}#/properties/settings")
            }}
        },
        "resources": [{"uri": RESOURCE, "schema": native_schema}]
    }))?;
    let schemas = Schemas::compose([module])?;

    let supplied = daml::parse(
        r#"!vendor.briefing.Item {
          name: "release"
          goal: "Extract the release name."
        }"#,
    )?;
    let resolved = schemas.resolve(&supplied)?;
    if resolved.root().get("settings").unwrap().tag() != Some("vendor.briefing.Settings") {
        return Err(io::Error::other("default settings tag was not applied").into());
    }
    let mut output = io::stdout().lock();
    serde_json::to_writer_pretty(&mut output, &resolved.to_json())?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

fn adapt_vendor_schema(mut schema: Value) -> Result<Value, &'static str> {
    let object = schema
        .as_object_mut()
        .ok_or("vendor schema must be an object")?;
    match object.remove("kind") {
        Some(Value::String(kind)) if kind == "record" => {
            object.insert("type".into(), Value::String("object".into()));
        }
        Some(_) => return Err("unsupported vendor schema kind"),
        None => {}
    }

    if let Some(tag) = object.remove("default_tag") {
        object.insert(
            "x-daml-default-tags".into(),
            json!([{"path":"", "tag":tag}]),
        );
    }

    if let Some(properties) = object.get_mut("properties") {
        let fields = properties
            .as_object_mut()
            .ok_or("vendor properties must be an object")?;
        for field_schema in fields.values_mut() {
            *field_schema = adapt_vendor_schema(std::mem::take(field_schema))?;
        }
    }
    Ok(schema)
}
