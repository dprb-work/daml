//! Inspect a parsed DAML file's values, tags, and original byte spans.

use daml::{Document, Node, Value};
use std::{env, error::Error, fs, io, process::ExitCode};

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
    let path = arguments
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "usage: inspect <file.daml>"))?;
    if arguments.next().is_some() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "expected one input file").into());
    }
    let document = daml::parse(fs::read_to_string(path)?)?;
    show(&document, document.root(), 0);
    Ok(())
}

fn pad(depth: usize) {
    print!("{:width$}", "", width = depth * 2);
}

fn show(document: &Document, node: &Node, depth: usize) {
    pad(depth);
    if let Some(tag) = document.tag(node) {
        print!("!{tag} ");
    }
    print!("@{}..{} ", node.span.start, node.span.end);
    // These nodes and source spans were produced together by the parser.
    match &node.value {
        Value::Null => println!("null"),
        Value::Bool(value) => println!("{value}"),
        Value::Number(span) => println!("number {}", document.slice(*span).unwrap()),
        Value::String(_) => println!("string {:?}", document.string(node).unwrap()),
        Value::Sequence(items) => {
            println!("sequence");
            for item in items {
                show(document, item, depth + 1);
            }
        }
        Value::Mapping(entries) => {
            println!("mapping");
            for entry in entries {
                pad(depth + 1);
                println!("{}:", document.slice(entry.key).unwrap());
                show(document, &entry.value, depth + 2);
            }
        }
    }
}
