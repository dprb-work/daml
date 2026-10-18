//! Reproducible parse/resolve measurements. Run `scripts/measure --help` for the CLI.

use daml::{ResolutionLimits, SchemaModule, Schemas};
use serde_json::{Value as Json, json};
use std::{
    env,
    error::Error,
    fs,
    hint::black_box,
    io,
    process::{Command, ExitCode},
    time::Instant,
};

const WARMUP: usize = 3;
const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;
const SUITE: &[(&str, &[usize])] = &[
    ("config", &[4, 32, 256]),
    ("literal", &[1024, 16384, 262144]),
    ("wrapped", &[1024, 16384, 262144]),
    ("deep", &[4, 16, 64]),
];

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
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--help" => {
            println!(
                "usage: measure suite [iterations] [repetitions]\n       measure worker <parse|resolve> <config|literal|wrapped|deep> <size> <iterations>\nSuite defaults: 1000 iterations, 5 fresh-process repetitions; each worker uses 3 warmups."
            );
            Ok(())
        }
        [mode] if mode == "suite" => suite(1000, 5),
        [mode, iterations] if mode == "suite" => {
            suite(parse_positive(iterations, "iterations")?, 5)
        }
        [mode, iterations, repetitions] if mode == "suite" => suite(
            parse_positive(iterations, "iterations")?,
            parse_positive(repetitions, "repetitions")?,
        ),
        [mode, operation, case, size, iterations] if mode == "worker" => {
            let operation = operation.as_str();
            if operation != "parse" && operation != "resolve" {
                return invalid("operation must be parse or resolve");
            }
            let size = parse_positive(size, "size")?;
            let iterations = parse_positive(iterations, "iterations")?;
            validate_size(case, size)?;
            worker(operation, case, size, iterations)
        }
        _ => invalid(
            "usage: measure suite [iterations] [repetitions] | measure worker <parse|resolve> <config|literal|wrapped|deep> <size> <iterations>",
        ),
    }
}

fn invalid<T>(message: &str) -> Result<T, Box<dyn Error>> {
    Err(io::Error::new(io::ErrorKind::InvalidInput, message).into())
}

fn parse_positive(value: &str, label: &str) -> Result<usize, Box<dyn Error>> {
    let parsed = value.parse::<usize>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} must be a positive integer"),
        )
    })?;
    if parsed == 0 {
        return invalid(&format!("{label} must be positive"));
    }
    Ok(parsed)
}

fn validate_size(case: &str, size: usize) -> Result<(), Box<dyn Error>> {
    let allowed = SUITE
        .iter()
        .find(|(name, _)| *name == case)
        .map(|(_, sizes)| *sizes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unknown case"))?;
    if !allowed.contains(&size) {
        return invalid("size must be one of the fixed suite sizes");
    }
    Ok(())
}

fn suite(iterations: usize, repetitions: usize) -> Result<(), Box<dyn Error>> {
    let executable = env::current_exe()?;
    let metadata = metadata()?;
    let capacity = 24usize
        .checked_mul(repetitions)
        .ok_or_else(|| io::Error::other("sample count overflows usize"))?;
    let mut samples = Vec::new();
    samples.try_reserve_exact(capacity)?;
    for (case, sizes) in SUITE {
        for &size in *sizes {
            for operation in ["parse", "resolve"] {
                for repetition in 0..repetitions {
                    let output = Command::new(&executable)
                        .args([
                            "worker",
                            operation,
                            case,
                            &size.to_string(),
                            &iterations.to_string(),
                        ])
                        .output()?;
                    if !output.status.success() {
                        return Err(io::Error::other(format!(
                            "worker {operation}/{case}/{size} failed: {}",
                            String::from_utf8_lossy(&output.stderr)
                        ))
                        .into());
                    }
                    let mut sample: Json = serde_json::from_slice(&output.stdout)?;
                    sample["repetition"] = json!(repetition + 1);
                    samples.push(sample);
                }
            }
        }
    }
    let record = json!({
        "format": "daml-measure-v1", "metadata": metadata,
        "suite": {"iterations": iterations, "warmup_iterations": WARMUP, "repetitions": repetitions,
                  "workers": samples.len(), "argv": env::args().collect::<Vec<_>>()},
        "samples": samples,
    });
    println!("{}", serde_json::to_string(&record)?);
    Ok(())
}

fn worker(
    operation: &str,
    case: &str,
    size: usize,
    iterations: usize,
) -> Result<(), Box<dyn Error>> {
    if !cfg!(target_os = "linux") {
        return invalid("measurement requires Linux VmHWM from /proc/self/status");
    }
    let source = source(case, size)?;
    if source.len() > MAX_SOURCE_BYTES {
        return invalid("generated source exceeds parser source limit");
    }
    let (schema, document) = if operation == "resolve" {
        let module = SchemaModule::from_json(&schema_json(case))?;
        let schemas = Schemas::compose([module])?;
        let parsed = daml::parse(source.as_str())?;
        (Some(schemas), Some(parsed))
    } else {
        (None, None)
    };
    let expected = size;
    if operation == "parse" {
        validate_parsed(case, size, &source)?;
    } else {
        validate_resolved(
            case,
            size,
            schema.as_ref().unwrap(),
            document.as_ref().unwrap(),
        )?;
    }
    let measure = || -> Result<(), Box<dyn Error>> {
        match operation {
            "parse" => {
                let doc = daml::parse(source.as_str())?;
                black_box(doc);
            }
            "resolve" => {
                let resolved = schema.as_ref().unwrap().resolve_with_limits(
                    document.as_ref().unwrap(),
                    None,
                    ResolutionLimits::default(),
                )?;
                black_box(resolved);
            }
            _ => unreachable!(),
        }
        Ok(())
    };
    for _ in 0..WARMUP {
        measure()?;
    }
    let start = Instant::now();
    for _ in 0..iterations {
        measure()?;
    }
    let elapsed = start.elapsed().as_nanos();
    let peak_rss_kib = vm_hwm_kib()?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "operation": operation, "case": case, "size": size, "iterations": iterations,
            "warmup_iterations": WARMUP, "elapsed_ns": elapsed,
            "ns_per_operation": elapsed / iterations as u128,
            "source_bytes": source.len(), "decoded_content_bytes_or_depth": expected,
            "peak_rss_kib": peak_rss_kib,
        }))?
    );
    Ok(())
}

fn source(case: &str, size: usize) -> Result<String, Box<dyn Error>> {
    let text = match case {
        "config" => {
            let mut source = String::from("!fuzz.Config { entries: [\n");
            for index in 0..size {
                source.push_str(&format!("!fuzz.Record {{ value: \"record-{index}\" }}\n"));
            }
            source.push_str("] }");
            source
        }
        "literal" => {
            let expected = expected_text(case, size);
            let mut source = String::from("!fuzz.Text \"\"\"\n");
            for line in expected.split('\n') {
                source.push_str("    ");
                source.push_str(line);
                source.push('\n');
            }
            source.push_str("\"\"\"");
            source
        }
        "wrapped" => {
            let expected = expected_text(case, size);
            let mut source = String::from("!fuzz.Wrapped {\n content: '''\n");
            let mut remaining = expected.as_str();
            while remaining.len() > 64 {
                let split = remaining[..64]
                    .rfind(' ')
                    .expect("fixed ASCII word pattern");
                source.push_str("    ");
                source.push_str(&remaining[..split]);
                source.push('\n');
                remaining = &remaining[split + 1..];
            }
            source.push_str("    ");
            source.push_str(remaining);
            source.push_str("\n '''\n}");
            source
        }
        "deep" => {
            let mut source = String::new();
            for _ in 0..size {
                source.push_str("!fuzz.Node { child: ");
            }
            source.push_str("!fuzz.Text \"leaf\"");
            for _ in 0..size {
                source.push_str(" }");
            }
            source
        }
        _ => return invalid("unknown case"),
    };
    if text.len() > MAX_SOURCE_BYTES {
        return invalid("generated source exceeds parser source limit");
    }
    Ok(text)
}

fn expected_text(case: &str, size: usize) -> String {
    let pattern = if case == "literal" {
        "line\n  indented\n"
    } else {
        "word "
    };
    let mut text = pattern.repeat(size.div_ceil(pattern.len()));
    text.truncate(size);
    if case == "wrapped" && text.ends_with(' ') {
        text.pop();
        text.push('x');
    }
    text
}

fn schema_json(_case: &str) -> String {
    let node = json!({"type":"object","properties":{"child":true},"additionalProperties":false});
    let mut types = serde_json::Map::new();
    let mut add = |name: &str, schema: Json| {
        types.insert(name.to_owned(), json!({"schema":schema}));
    };
    add(
        "fuzz.Config",
        json!({"type":"object","properties":{"label":{"type":"string","default":"configured"},"entries":{"type":"array","items":{"type":"object","properties":{"value":{"type":"string"}},"additionalProperties":true}}},"required":["label","entries"],"additionalProperties":true}),
    );
    add(
        "fuzz.Record",
        json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":true}),
    );
    add("fuzz.Text", json!({"type":"string"}));
    add(
        "fuzz.Wrapped",
        json!({"type":"object","properties":{"content":{"type":"string"}},"required":["content"],"additionalProperties":true}),
    );
    add("fuzz.Node", node);
    json!({"types":types}).to_string()
}

fn validate_parsed(case: &str, size: usize, source: &str) -> Result<(), Box<dyn Error>> {
    let document = daml::parse(source)?;
    match case {
        "literal" => {
            let expected = expected_text(case, size);
            if document.string(document.root()) != Some(expected.as_str())
                || document.tag(document.root()) != Some("fuzz.Text")
            {
                return invalid("literal parsed value/tag mismatch");
            }
        }
        "wrapped" => {
            let expected = expected_text(case, size);
            let root = document.root();
            let value = document
                .get(root, "content")
                .ok_or_else(|| io::Error::other("wrapped content missing"))?;
            if document.string(value) != Some(expected.as_str())
                || document.tag(root) != Some("fuzz.Wrapped")
            {
                return invalid("wrapped parsed value/tag mismatch");
            }
        }
        "config" => {
            let entries = document
                .get(document.root(), "entries")
                .ok_or_else(|| io::Error::other("config entries missing"))?;
            let daml::Value::Sequence(entries) = &entries.value else {
                return invalid("config entries are not a list");
            };
            if entries.len() != size {
                return invalid("config record count mismatch");
            }
            for (index, node) in entries.iter().enumerate() {
                let value = document
                    .get(node, "value")
                    .ok_or_else(|| io::Error::other("config record value missing"))?;
                if document.tag(node) != Some("fuzz.Record")
                    || document.string(value) != Some(format!("record-{index}").as_str())
                {
                    return invalid("config record value/tag mismatch");
                }
            }
        }
        "deep" => {
            let mut node = document.root();
            for _ in 0..size {
                if document.tag(node) != Some("fuzz.Node") {
                    return invalid("deep node tag mismatch");
                }
                node = document
                    .get(node, "child")
                    .ok_or_else(|| io::Error::other("deep child missing"))?;
            }
            if document.tag(node) != Some("fuzz.Text") || document.string(node) != Some("leaf") {
                return invalid("deep leaf mismatch");
            }
        }
        _ => return invalid("unknown case"),
    }
    Ok(())
}

fn validate_resolved(
    case: &str,
    size: usize,
    schemas: &Schemas,
    document: &daml::Document,
) -> Result<(), Box<dyn Error>> {
    let resolved = schemas.resolve(document)?;
    match case {
        "config" => {
            let label = resolved
                .root()
                .get("label")
                .ok_or_else(|| io::Error::other("resolved config default missing"))?;
            if label.string() != Some("configured") {
                return invalid("resolved config default mismatch");
            }
            let entries = resolved
                .root()
                .get("entries")
                .and_then(|entries| entries.items())
                .ok_or_else(|| io::Error::other("resolved config entries are not a list"))?;
            if entries.len() != size {
                return invalid("resolved record count mismatch");
            }
            for (index, record) in entries.iter().enumerate() {
                if record.tag() != Some("fuzz.Record")
                    || record.get("value").and_then(|value| value.string())
                        != Some(format!("record-{index}").as_str())
                {
                    return invalid("resolved config value/tag mismatch");
                }
            }
        }
        "literal" => {
            let expected = expected_text(case, size);
            if resolved.root().tag() != Some("fuzz.Text")
                || resolved.root().string() != Some(expected.as_str())
            {
                return invalid("resolved literal value/tag mismatch");
            }
        }
        "wrapped" => {
            let expected = expected_text(case, size);
            let value = resolved
                .root()
                .get("content")
                .ok_or_else(|| io::Error::other("resolved wrapped content missing"))?;
            if resolved.root().tag() != Some("fuzz.Wrapped")
                || value.string() != Some(expected.as_str())
            {
                return invalid("resolved wrapped content/tag mismatch");
            }
        }
        "deep" => {
            let mut node = resolved.root();
            for _ in 0..size {
                if node.tag() != Some("fuzz.Node") {
                    return invalid("resolved deep node tag mismatch");
                }
                node = node
                    .get("child")
                    .ok_or_else(|| io::Error::other("resolved deep child missing"))?;
            }
            if node.tag() != Some("fuzz.Text") || node.string() != Some("leaf") {
                return invalid("resolved deep leaf mismatch");
            }
        }
        _ => return invalid("unknown case"),
    }
    Ok(())
}

fn vm_hwm_kib() -> Result<u64, Box<dyn Error>> {
    let status = fs::read_to_string("/proc/self/status")?;
    let line = status
        .lines()
        .find(|line| line.starts_with("VmHWM:"))
        .ok_or_else(|| io::Error::other("VmHWM missing from /proc/self/status"))?;
    let mut fields = line.split_whitespace();
    let _name = fields.next();
    let value = fields
        .next()
        .ok_or_else(|| io::Error::other("VmHWM value missing"))?
        .parse::<u64>()?;
    if fields.next() != Some("kB") {
        return Err(io::Error::other("VmHWM unit is not kB").into());
    }
    Ok(value)
}

fn metadata() -> Result<Json, Box<dyn Error>> {
    let head = command_text("git", &["rev-parse", "HEAD"])?;
    let dirty = Command::new("git")
        .args([
            "status",
            "--porcelain",
            "--",
            "*.rs",
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "scripts/measure",
        ])
        .output()?;
    if !dirty.status.success() {
        return invalid("git status failed; cannot establish Rust source cleanliness");
    }
    let dirty_text = String::from_utf8(dirty.stdout)?;
    if !dirty_text.trim().is_empty() {
        return invalid(
            "measurement sources or build inputs are dirty; commit them before measuring",
        );
    }
    let rustc = command_text("rustc", &["-Vv"])?;
    let kernel = command_text("uname", &["-sr"])?;
    let cpuinfo = fs::read_to_string("/proc/cpuinfo")?;
    let cpu = cpuinfo
        .lines()
        .find(|line| line.starts_with("model name"))
        .and_then(|line| line.split_once(':').map(|(_, v)| v.trim()))
        .ok_or_else(|| io::Error::other("CPU model missing from /proc/cpuinfo"))?;
    let affinity = fs::read_to_string("/proc/self/status")?
        .lines()
        .find(|line| line.starts_with("Cpus_allowed_list:"))
        .and_then(|line| line.split_once(':').map(|(_, v)| v.trim().to_owned()))
        .ok_or_else(|| io::Error::other("CPU affinity missing from /proc/self/status"))?;
    Ok(
        json!({"git_head":head.trim(),"source_dirty":false,"rustc_vv":rustc.trim(),"kernel":kernel.trim(),"cpu_model":cpu,"logical_cpus":std::thread::available_parallelism()?.get(),"allowed_cpu_affinity":affinity,"timestamp_utc":command_text("date", &["-u","+%Y-%m-%dT%H:%M:%SZ"])?.trim(),"platform":env::consts::OS,"architecture":env::consts::ARCH,"debug_assertions":cfg!(debug_assertions),"rustflags":env::var("RUSTFLAGS").ok(),"cargo_encoded_rustflags":env::var("CARGO_ENCODED_RUSTFLAGS").ok()}),
    )
}

fn command_text(command: &str, args: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = Command::new(command).args(args).output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!("{command} command failed")).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}
