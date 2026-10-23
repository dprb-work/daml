//! Format DAML documents from a file or standard input.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::process::ExitCode;

use daml::{FormatOptions, Limits, format_definition, parse_definition};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("daml-fmt: {message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<u8, String> {
    let mut check = false;
    let mut width = FormatOptions::default().line_width;
    let mut width_seen = false;
    let mut input_path: Option<OsString> = None;
    let mut args = std::env::args_os().skip(1);

    while let Some(arg) = args.next() {
        if arg == "--help" {
            if check || width_seen || input_path.is_some() || args.next().is_some() {
                return Err("--help cannot be combined with other arguments".into());
            }
            println!(
                "Usage: daml-fmt [--check] [--width N] [FILE|-]\n\nFormat one DAML document; read standard input when FILE is omitted or '-'.\n--check exits 1 when formatting would change the input."
            );
            return Ok(0);
        } else if arg == "--check" {
            if check {
                return Err("repeated option --check".into());
            }
            check = true;
        } else if arg == "--width" {
            if width_seen {
                return Err("repeated option --width".into());
            }
            width_seen = true;
            let value = args
                .next()
                .ok_or_else(|| "--width requires a value".to_owned())?;
            let text = value
                .to_str()
                .ok_or_else(|| "width must be a positive integer".to_owned())?;
            width = text
                .parse()
                .map_err(|_| "width must be a positive integer".to_owned())?;
        } else if arg.to_string_lossy().starts_with('-') && arg != "-" {
            return Err(format!("unknown option {}", arg.to_string_lossy()));
        } else if input_path.replace(arg).is_some() {
            return Err("expected at most one input file".into());
        }
    }

    let options = FormatOptions { line_width: width };
    options.validate().map_err(|error| error.to_string())?;
    let input = read_input(input_path.as_deref())?;
    let definition = parse_definition(input).map_err(|error| error.to_string())?;
    let output = format_definition(&definition, options).map_err(|error| error.to_string())?;
    if check {
        if output == definition.source() {
            Ok(0)
        } else {
            let label = input_path
                .as_deref()
                .filter(|path| *path != "-")
                .map(std::path::Path::new);
            match label {
                Some(path) => eprintln!("daml-fmt: would reformat {}", path.display()),
                None => eprintln!("daml-fmt: would reformat standard input"),
            }
            Ok(1)
        }
    } else {
        io::stdout()
            .write_all(output.as_bytes())
            .map_err(|error| error.to_string())?;
        Ok(0)
    }
}

fn read_input(path: Option<&std::ffi::OsStr>) -> Result<String, String> {
    let limit = Limits::default().max_input_bytes;
    let mut bytes = Vec::new();
    match path {
        Some(path) if path != "-" => {
            let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
            read_bounded(file, limit, &mut bytes)?;
        }
        _ => read_bounded(io::stdin(), limit, &mut bytes)?,
    }
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

fn read_bounded(reader: impl Read, limit: usize, bytes: &mut Vec<u8>) -> Result<(), String> {
    reader
        .take(limit.saturating_add(1) as u64)
        .read_to_end(bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err(format!("input exceeds the {limit}-byte limit"));
    }
    Ok(())
}
