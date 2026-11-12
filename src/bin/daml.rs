//! Check, resolve, and explain configuration through DAML's core engine.

use daml::{
    Bundle, Document, Explanation, ExplanationConstraint, FormatOptions, Limits, Origin,
    ResolveError, ResolvedNode, ResolvedValue, SchemaModule, Schemas, SemanticNode, SemanticValue,
    SourceEdit, SourceEditLimits, Value,
};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use std::{
    env,
    error::Error,
    ffi::OsString,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

const MAX_SCHEMA_BYTES: usize = 8 * 1024 * 1024;
const MAX_MODULES: usize = 10_000;
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const HELP: &str = r#"Usage: daml <check|resolve|explain|edit> [OPTIONS] [FILE|-]

Check configuration, resolve defaults, explain a checked value, or edit authored source.
Read stdin when FILE is omitted or '-'. Input files are never changed.

Options:
  --bundle DIR     Select a native bundle (otherwise discover from FILE).
  --module FILE    Compose a JSON schema-module envelope; repeat for resources.
                   At most 10,000 files and 8 MiB of total module source.
                   Cannot be combined with --bundle. No resource fetching.
  --expected NAME  Require this root contract alongside any explicit assertion.
  --format FORMAT  resolve only: daml (default) or json.
                   JSON omits assertions and projects temporal/cron kinds to strings;
                   numeric values remain exact.
  --path POINTER   explain only: RFC 6901 path (default: empty root pointer).
  --edits FILE     edit only: a DAML sequence of replace/insert/remove operation maps.
                   Requires op/path strings and value for replace/insert; '-' is forbidden.
  --               Treat remaining arguments as file names.
  --help           Show this help, alone or immediately after an operation.

A final root assertion or --expected is required. Stdin requires --bundle or --module.
check writes no successful data. resolve writes completed data; explain writes
JSON containing semantic values, origins, assertions, and constraint locations.
edit validates the changed document and writes authored source without omitted defaults.
Output is bounded to 8 MiB; normal engine admission/work limits also apply.

Exit status: 0 success; 1 configuration/schema/admission/path/representation error;
2 usage or I/O error. Diagnostics go to stderr; successful results go to stdout.
"#;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Check,
    Resolve,
    Explain,
    Edit,
}

#[derive(Clone, Copy)]
enum OutputFormat {
    Daml,
    Json,
}

struct Options {
    operation: Operation,
    input: Option<PathBuf>,
    bundle: Option<PathBuf>,
    modules: Vec<PathBuf>,
    expected: Option<String>,
    format: OutputFormat,
    pointer: String,
    edits: Option<PathBuf>,
}

enum CommandOutput {
    Bytes(Vec<u8>),
    Edited(Document),
}

impl CommandOutput {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Bytes(bytes) => bytes,
            Self::Edited(document) => document.source().as_bytes(),
        }
    }
}

enum EditInput {
    Replace { path: String, value: SemanticNode },
    Insert { path: String, value: SemanticNode },
    Remove { path: String },
}

struct Failure {
    status: u8,
    message: String,
}

impl Failure {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            status: 2,
            message: message.into(),
        }
    }

    fn configuration(message: impl Into<String>) -> Self {
        Self {
            status: 1,
            message: message.into(),
        }
    }

    // Presentation is the executable boundary: retain the original cause chain
    // and schema location rather than reducing engine errors to their headline.
    fn engine(error: impl Error + 'static) -> Self {
        let mut message = error.to_string();
        let mut status = 1;
        let mut current: &(dyn Error + 'static) = &error;
        loop {
            if current.is::<io::Error>() {
                status = 2;
            }
            if let Some(error) = current.downcast_ref::<ResolveError>()
                && let Some(schema) = &error.schema
            {
                message.push_str(&format!(" [schema {}#{}]", schema.resource, schema.pointer));
            }
            match current.source() {
                Some(cause) => {
                    message.push_str(": ");
                    message.push_str(&cause.to_string());
                    current = cause;
                }
                None => break,
            }
        }
        Self { status, message }
    }
}

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let first = args.next();
    let result = if first.as_deref() == Some(std::ffi::OsStr::new("--version")) {
        if args.next().is_none() {
            Ok(Some(CommandOutput::Bytes(
                format!("daml {}\n", env!("CARGO_PKG_VERSION")).into_bytes(),
            )))
        } else {
            Err(Failure::usage(
                "--version cannot be combined with other arguments",
            ))
        }
    } else {
        parse_options(first.into_iter().chain(args))
            .and_then(|options| options.map(execute).transpose())
    };
    let result = result.and_then(|output| {
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(
                output
                    .as_ref()
                    .map_or(HELP.as_bytes(), CommandOutput::as_bytes),
            )
            .and_then(|()| stdout.flush())
            .map_err(|error| Failure::usage(format!("stdout: {error}")))
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            let _ = writeln!(io::stderr().lock(), "daml: {}", failure.message);
            ExitCode::from(failure.status)
        }
    }
}

fn parse_options(mut args: impl Iterator<Item = OsString>) -> Result<Option<Options>, Failure> {
    let command = args
        .next()
        .ok_or_else(|| Failure::usage("expected check, resolve, explain, or edit; use --help"))?;
    if command == "--help" && args.next().is_none() {
        return Ok(None);
    }
    let operation = match command.to_str() {
        Some("check") => Operation::Check,
        Some("resolve") => Operation::Resolve,
        Some("explain") => Operation::Explain,
        Some("edit") => Operation::Edit,
        _ => {
            return Err(Failure::usage(
                "expected check, resolve, explain, or edit; use --help",
            ));
        }
    };
    let mut options = Options {
        operation,
        input: None,
        bundle: None,
        modules: Vec::new(),
        expected: None,
        format: OutputFormat::Daml,
        pointer: String::new(),
        edits: None,
    };
    let mut positional = false;
    let mut format_seen = false;
    let mut path_seen = false;
    let mut seen_argument = false;
    while let Some(argument) = args.next() {
        if !positional && argument == "--help" && !seen_argument && args.next().is_none() {
            return Ok(None);
        }
        seen_argument = true;
        if !positional && argument == "--" {
            positional = true;
        } else if !positional && argument == "--bundle" {
            let value = option_value(&mut args, "--bundle")?;
            if options.bundle.replace(value.into()).is_some() {
                return Err(Failure::usage("repeated --bundle"));
            }
        } else if !positional && argument == "--module" {
            if options.modules.len() == MAX_MODULES {
                return Err(Failure::usage("at most 10,000 --module files are allowed"));
            }
            options
                .modules
                .push(option_value(&mut args, "--module")?.into());
        } else if !positional && argument == "--expected" {
            let value = option_text(&mut args, "--expected")?;
            if value.is_empty() || options.expected.replace(value).is_some() {
                return Err(Failure::usage("--expected requires one nonempty name"));
            }
        } else if !positional && argument == "--format" {
            if operation != Operation::Resolve || format_seen {
                return Err(Failure::usage("--format is allowed once, on resolve only"));
            }
            format_seen = true;
            options.format = match option_text(&mut args, "--format")?.as_str() {
                "daml" => OutputFormat::Daml,
                "json" => OutputFormat::Json,
                _ => return Err(Failure::usage("--format must be daml or json")),
            };
        } else if !positional && argument == "--path" {
            if operation != Operation::Explain || path_seen {
                return Err(Failure::usage("--path is allowed once, on explain only"));
            }
            path_seen = true;
            options.pointer = option_text(&mut args, "--path")?;
        } else if !positional && argument == "--edits" {
            if operation != Operation::Edit || options.edits.is_some() {
                return Err(Failure::usage("--edits is required once, on edit only"));
            }
            let value = option_value(&mut args, "--edits")?;
            if value == "-" {
                return Err(Failure::usage("--edits cannot read stdin"));
            }
            options.edits = Some(value.into());
        } else if !positional && argument != "-" && argument.as_encoded_bytes().starts_with(b"-") {
            return Err(Failure::usage(format!(
                "unknown option {}",
                argument.to_string_lossy()
            )));
        } else if options.input.replace(argument.into()).is_some() {
            return Err(Failure::usage("expected at most one input file"));
        }
    }
    if options.bundle.is_some() && !options.modules.is_empty() {
        return Err(Failure::usage(
            "--bundle and --module are mutually exclusive",
        ));
    }
    if options.input.as_deref() == Some(Path::new("-")) {
        options.input = None;
    }
    if operation == Operation::Edit && options.edits.is_none() {
        return Err(Failure::usage("edit requires --edits FILE"));
    }
    Ok(Some(options))
}

fn option_value(
    args: &mut impl Iterator<Item = OsString>,
    name: &str,
) -> Result<OsString, Failure> {
    match args.next() {
        Some(value) if !value.as_encoded_bytes().starts_with(b"--") => Ok(value),
        _ => Err(Failure::usage(format!("{name} requires a value"))),
    }
}

fn option_text(args: &mut impl Iterator<Item = OsString>, name: &str) -> Result<String, Failure> {
    option_value(args, name)?
        .into_string()
        .map_err(|_| Failure::usage(format!("{name} requires UTF-8 text")))
}

fn read_text(path: Option<&Path>, limit: usize) -> Result<String, Failure> {
    let label = path.map_or_else(|| "stdin".into(), |path| path.display().to_string());
    let mut bytes = Vec::new();
    let result = match path {
        Some(path) => {
            File::open(path).and_then(|file| file.take(limit as u64 + 1).read_to_end(&mut bytes))
        }
        None => io::stdin().take(limit as u64 + 1).read_to_end(&mut bytes),
    };
    result.map_err(|error| Failure::usage(format!("{label}: {error}")))?;
    if bytes.len() > limit {
        return Err(Failure::configuration(format!(
            "{label}: input exceeds {limit} bytes"
        )));
    }
    String::from_utf8(bytes).map_err(|error| Failure::configuration(format!("{label}: {error}")))
}

fn edit_text(node: Option<SemanticNode>, name: &str, index: usize) -> Result<String, Failure> {
    let node = node
        .ok_or_else(|| Failure::configuration(format!("edit operation {index}: missing {name}")))?;
    if node.tag.is_some() {
        return Err(Failure::configuration(format!(
            "edit operation {index}: {name} must be an untagged string"
        )));
    }
    match node.value {
        SemanticValue::String(value) => Ok(value),
        _ => Err(Failure::configuration(format!(
            "edit operation {index}: {name} must be an untagged string"
        ))),
    }
}

fn read_edits(path: &Path, max_operations: usize) -> Result<Vec<EditInput>, Failure> {
    let source = read_text(Some(path), Limits::default().max_input_bytes)?;
    let document = daml::parse(source).map_err(Failure::engine)?;
    let Value::Sequence(items) = &document.root().value else {
        return Err(Failure::configuration(
            "edits must be an untagged DAML sequence",
        ));
    };
    if document.root().tag.is_some() {
        return Err(Failure::configuration(
            "edits must be an untagged DAML sequence",
        ));
    }
    if items.len() > max_operations {
        return Err(Failure::configuration("edit batch exceeds operation limit"));
    }
    // Project the admitted batch once, then move its owned values into requests.
    // Do not copy/reparse each value fragment or convert the configuration tree.
    let SemanticValue::Sequence(items) = SemanticNode::from_document(&document).value else {
        unreachable!("an admitted sequence projects as a semantic sequence");
    };
    let mut edits = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        let SemanticValue::Mapping(mut fields) = item.value else {
            return Err(Failure::configuration(format!(
                "edit operation {index}: expected an untagged mapping"
            )));
        };
        if item.tag.is_some() {
            return Err(Failure::configuration(format!(
                "edit operation {index}: expected an untagged mapping"
            )));
        }
        if fields
            .keys()
            .any(|name| !matches!(name.as_str(), "op" | "path" | "value"))
        {
            return Err(Failure::configuration(format!(
                "edit operation {index}: unknown field"
            )));
        }
        let op = edit_text(fields.remove("op"), "op", index)?;
        let path = edit_text(fields.remove("path"), "path", index)?;
        let value = fields.remove("value");
        let edit = match op.as_str() {
            "replace" | "insert" => {
                let value = value.ok_or_else(|| {
                    Failure::configuration(format!("edit operation {index}: missing value"))
                })?;
                if op == "replace" {
                    EditInput::Replace { path, value }
                } else {
                    EditInput::Insert { path, value }
                }
            }
            "remove" => {
                if value.is_some() {
                    return Err(Failure::configuration(format!(
                        "edit operation {index}: remove forbids value"
                    )));
                }
                EditInput::Remove { path }
            }
            _ => {
                return Err(Failure::configuration(format!(
                    "edit operation {index}: unknown operation"
                )));
            }
        };
        edits.push(edit);
    }
    Ok(edits)
}

fn execute(options: Options) -> Result<CommandOutput, Failure> {
    if options.input.is_none() && options.bundle.is_none() && options.modules.is_empty() {
        return Err(Failure::configuration(
            "stdin requires --bundle or --module",
        ));
    }
    let source = read_text(options.input.as_deref(), Limits::default().max_input_bytes)?;
    let document = daml::parse(source).map_err(Failure::engine)?;
    let document = if options.operation == Operation::Edit {
        let limits = SourceEditLimits::default();
        let inputs = read_edits(
            options.edits.as_deref().expect("edit flag was admitted"),
            limits.max_operations,
        )?;
        let edits: Vec<_> = inputs
            .iter()
            .map(|input| match input {
                EditInput::Replace { path, value } => SourceEdit::Replace { path, value },
                EditInput::Insert { path, value } => SourceEdit::Insert { path, value },
                EditInput::Remove { path } => SourceEdit::Remove { path },
            })
            .collect();
        document
            .prepare_source_edits(&edits, FormatOptions::default(), limits)
            .map_err(Failure::engine)?
            .apply(document.source())
            .map_err(Failure::engine)?
    } else {
        document
    };
    if document.tag(document.root()).is_none() && options.expected.is_none() {
        return Err(Failure::configuration(
            "a root assertion or --expected contract is required",
        ));
    }
    let schemas = if options.modules.is_empty() {
        let bundle = match &options.bundle {
            Some(path) => Bundle::open(path).map_err(Failure::engine)?,
            None => Bundle::discover(options.input.as_ref().expect("stdin context checked above"))
                .map_err(Failure::engine)?
                .ok_or_else(|| {
                    Failure::configuration("no ancestor daml.bundle; select --bundle or --module")
                })?,
        };
        bundle
            .compile(&document, options.expected.as_deref())
            .map_err(Failure::engine)?
    } else {
        let mut modules = Vec::new();
        let mut remaining = MAX_SCHEMA_BYTES;
        for path in &options.modules {
            let source = read_text(Some(path), remaining)?;
            remaining -= source.len();
            modules.push(SchemaModule::from_json(&source).map_err(Failure::engine)?);
        }
        Schemas::compose(modules).map_err(Failure::engine)?
    };
    let expected = options.expected.as_deref();
    if options.operation == Operation::Explain {
        let explanation = schemas
            .explain(&document, expected, &options.pointer)
            .map_err(Failure::engine)?;
        return json_output(&Report {
            explanation: &explanation,
            source: options.input.as_deref(),
        })
        .map(CommandOutput::Bytes);
    }
    let resolved = schemas
        .resolve_with_limits(&document, expected, Default::default())
        .map_err(Failure::engine)?;
    if options.operation == Operation::Edit {
        drop(resolved);
        return Ok(CommandOutput::Edited(document));
    }
    if options.operation == Operation::Check {
        return Ok(CommandOutput::Bytes(Vec::new()));
    }
    match options.format {
        OutputFormat::Json => json_output(&NodeOutput {
            node: resolved.root(),
            semantic: false,
        }),
        OutputFormat::Daml => schemas
            .format_semantic(
                &SemanticNode::from_resolved(resolved.root()),
                FormatOptions::default(),
            )
            .map(String::into_bytes)
            .map_err(Failure::engine),
    }
    .map(CommandOutput::Bytes)
}

struct BoundedOutput(Vec<u8>);
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_OUTPUT_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("output exceeds 8 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn json_output(value: &impl Serialize) -> Result<Vec<u8>, Failure> {
    let mut output = BoundedOutput(Vec::new());
    // These writes target a bounded memory buffer, not an external I/O stream.
    serde_json::to_writer_pretty(&mut output, value)
        .map_err(|error| Failure::configuration(error.to_string()))?;
    output
        .write_all(b"\n")
        .map_err(|error| Failure::configuration(error.to_string()))?;
    Ok(output.0)
}

struct Report<'r, 'a> {
    explanation: &'r Explanation<'a>,
    source: Option<&'r Path>,
}
impl Serialize for Report<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let node = self.explanation.node();
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("path", self.explanation.path())?;
        map.serialize_entry(
            "value",
            &NodeOutput {
                node,
                semantic: true,
            },
        )?;
        map.serialize_entry(
            "origin",
            &OriginOutput {
                origin: node.origin(),
                source: self.source,
            },
        )?;
        if let Some(origin) = node.tag_origin() {
            map.serialize_entry(
                "tagOrigin",
                &OriginOutput {
                    origin,
                    source: self.source,
                },
            )?;
        }
        map.serialize_entry("constraints", &Constraints(self.explanation.constraints()))?;
        map.end()
    }
}

struct OriginOutput<'a> {
    origin: Origin<'a>,
    source: Option<&'a Path>,
}
impl Serialize for OriginOutput<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self.origin {
            Origin::Input(span) => {
                map.serialize_entry("kind", "input")?;
                map.serialize_entry("span", &SpanOutput(span))?;
                if let Some(source) = self.source {
                    map.serialize_entry("source", &source.to_string_lossy())?;
                }
            }
            Origin::Default(location) => {
                map.serialize_entry("kind", "default")?;
                map.serialize_entry("resource", &location.resource)?;
                map.serialize_entry("pointer", &location.pointer)?;
            }
            Origin::Semantic => map.serialize_entry("kind", "semantic")?,
        }
        map.end()
    }
}
struct SpanOutput(daml::Span);
impl Serialize for SpanOutput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("start", &self.0.start)?;
        map.serialize_entry("end", &self.0.end)?;
        map.end()
    }
}
struct LocationOutput<'a>(&'a daml::SchemaLocation);
impl Serialize for LocationOutput<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("resource", &self.0.resource)?;
        map.serialize_entry("pointer", &self.0.pointer)?;
        map.end()
    }
}
struct Constraints<'r, 'a>(&'r [ExplanationConstraint<'a>]);
impl Serialize for Constraints<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for constraint in self.0 {
            sequence.serialize_element(&ConstraintOutput(constraint))?;
        }
        sequence.end()
    }
}
struct ConstraintOutput<'r, 'a>(&'r ExplanationConstraint<'a>);
impl Serialize for ConstraintOutput<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("instancePath", &self.0.instance_path)?;
        map.serialize_entry("schemaLocation", &LocationOutput(self.0.location))?;
        map.serialize_entry("schema", self.0.schema)?;
        map.end()
    }
}

// Serialize borrowed checked data directly: JSON output does not need an owned
// semantic tree, and exact numbers never travel through a floating-point value.
struct NodeOutput<'n, 'a> {
    node: &'n ResolvedNode<'a>,
    semantic: bool,
}
impl Serialize for NodeOutput<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let node = self.node;
        if !self.semantic {
            return match node.value() {
                ResolvedValue::Null => serializer.serialize_none(),
                ResolvedValue::Bool(value) => value.serialize(serializer),
                ResolvedValue::Number(value) => value.as_ref().serialize(serializer),
                ResolvedValue::String(value) => value.serialize(serializer),
                ResolvedValue::Temporal(value) => value.as_str().serialize(serializer),
                ResolvedValue::Sequence(items) => NodeList {
                    items,
                    semantic: false,
                }
                .serialize(serializer),
                ResolvedValue::Mapping(fields) => {
                    let mut map = serializer.serialize_map(Some(fields.len()))?;
                    for (key, node) in fields {
                        map.serialize_entry(
                            key,
                            &NodeOutput {
                                node,
                                semantic: false,
                            },
                        )?;
                    }
                    map.end()
                }
            };
        }
        let mut map = serializer.serialize_map(None)?;
        let kind = match node.value() {
            ResolvedValue::Null => "null",
            ResolvedValue::Bool(_) => "bool",
            ResolvedValue::Number(_) => "number",
            ResolvedValue::String(_) => "string",
            ResolvedValue::Temporal(value) => value.kind().as_str(),
            ResolvedValue::Sequence(_) => "list",
            ResolvedValue::Mapping(_) => "map",
        };
        map.serialize_entry("kind", kind)?;
        match node.value() {
            ResolvedValue::Null => {}
            ResolvedValue::Bool(value) => map.serialize_entry("value", value)?,
            ResolvedValue::Number(value) => {
                map.serialize_entry("value", value.as_ref().as_str())?
            }
            ResolvedValue::String(value) => map.serialize_entry("value", value)?,
            ResolvedValue::Temporal(value) => map.serialize_entry("value", value.as_str())?,
            ResolvedValue::Sequence(items) => map.serialize_entry(
                "items",
                &NodeList {
                    items,
                    semantic: true,
                },
            )?,
            ResolvedValue::Mapping(fields) => map.serialize_entry("entries", &Entries(fields))?,
        }
        if let Some(name) = node.tag() {
            map.serialize_entry(
                "tag",
                &TagOutput {
                    name,
                    binding: node.tag_binding(),
                },
            )?;
        }
        map.end()
    }
}
struct NodeList<'n, 'a> {
    items: &'n [ResolvedNode<'a>],
    semantic: bool,
}
impl Serialize for NodeList<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.items.len()))?;
        for node in self.items {
            sequence.serialize_element(&NodeOutput {
                node,
                semantic: self.semantic,
            })?;
        }
        sequence.end()
    }
}
struct Entries<'n, 'a>(&'n std::collections::BTreeMap<std::borrow::Cow<'a, str>, ResolvedNode<'a>>);
impl Serialize for Entries<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for (key, node) in self.0 {
            sequence.serialize_element(&EntryOutput { key, node })?;
        }
        sequence.end()
    }
}
struct EntryOutput<'n, 'a> {
    key: &'n str,
    node: &'n ResolvedNode<'a>,
}
impl Serialize for EntryOutput<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("key", self.key)?;
        map.serialize_entry(
            "value",
            &NodeOutput {
                node: self.node,
                semantic: true,
            },
        )?;
        map.end()
    }
}
struct TagOutput<'a> {
    name: &'a str,
    binding: Option<&'a str>,
}
impl Serialize for TagOutput<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("name", self.name)?;
        if let Some(binding) = self.binding {
            map.serialize_entry("binding", binding)?;
        }
        map.end()
    }
}
