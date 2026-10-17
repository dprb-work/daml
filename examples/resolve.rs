//! Complete defaults and check a DAML file against explicit JSON/DAML modules.

use daml::{SchemaModule, Schemas};
use std::{
    env,
    error::Error,
    fs,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

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
    let mut arguments = env::args_os().skip(1);
    let input = arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: resolve <input.daml> <module.daml|module.json>...",
        )
    })?;
    let mut modules = Vec::new();
    for argument in arguments {
        let path = PathBuf::from(argument);
        let source = fs::read_to_string(&path)?;
        let module = match path.extension().and_then(|extension| extension.to_str()) {
            Some("json") => SchemaModule::from_json(&source)?,
            Some("daml") => SchemaModule::from_daml(&source)?,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "schema module files must have an explicit .json or .daml extension",
                )
                .into());
            }
        };
        modules.push(module);
    }
    if modules.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one explicit schema module is required",
        )
        .into());
    }
    let schemas = Schemas::compose(modules)?;
    let document = daml::parse(fs::read_to_string(input)?)?;
    let resolved = schemas.resolve(&document)?;
    let mut output = io::stdout().lock();
    serde_json::to_writer_pretty(&mut output, &resolved.to_json())?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}
