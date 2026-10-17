//! Import an Olaf-shaped schema without importing or executing Olaf.
//!
//! The schema below is independently authored, not copied from Olaf's fixtures.

use daml::{SchemaModule, Schemas};
use std::{
    error::Error,
    io::{self, Write},
    process::ExitCode,
};

const SCHEMA: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$ref": "#/$defs/BriefingItem",
  "$defs": {
    "BriefingItem": {
      "type": "object",
      "properties": {
        "__type__": {"enum": ["olaf.llm.briefing.BriefingItem"]},
        "name": {"type": "string", "minLength": 1},
        "goal": {"type": "string", "minLength": 1},
        "active": {"type": "boolean", "default": true}
      },
      "required": ["__type__", "name", "goal", "active"],
      "additionalProperties": false
    }
  }
}"##;

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
    let module = SchemaModule::from_olaf_json("olaf.llm.briefing.BriefingItem", SCHEMA)?;
    let schemas = Schemas::compose([module])?;
    let supplied = daml::parse(
        r#"!olaf.llm.briefing.BriefingItem {
      name: "release"
      goal: '''
        Extract the release name.
        Return JSON only.
      '''
    }"#,
    )?;
    let resolved = schemas.resolve(&supplied)?;
    let mut output = io::stdout().lock();
    serde_json::to_writer_pretty(&mut output, &resolved.to_json())?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}
