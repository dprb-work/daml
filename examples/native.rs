//! Resolve a configuration through native definitions and private bundle imports.

use daml::Bundle;
use std::{
    env,
    error::Error,
    fs,
    io::{self, Write},
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
    let root = arguments.next().ok_or_else(usage)?;
    let input = arguments.next().ok_or_else(usage)?;
    if arguments.next().is_some() {
        return Err(usage().into());
    }
    let document = daml::parse(fs::read_to_string(input)?)?;
    let bundle = Bundle::open(root)?;
    let schemas = bundle.compile(&document, None)?;
    let resolved = schemas.resolve(&document)?;
    let mut output = io::stdout().lock();
    serde_json::to_writer_pretty(&mut output, &resolved.to_json())?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

fn usage() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "usage: native <bundle-directory> <input.daml>",
    )
}
