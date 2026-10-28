# Architecture and implementation boundaries

DAML's implementation is organized around explicit owners. Extend the existing owner when it can express the behavior; avoid parallel walkers, validators, formatters, and admission paths.

## Internal boundaries

- `src/schema/profile.rs` owns child-schema classification, traversal roles, and schema-admission limits. Add schema positions there, not in another walker.
- `src/schema.rs` admits local schema shape and declarations in `SchemaModule`. Private admitted values cannot be mutated by callers. Composition owns aggregate budgets, cross-module collisions and references, and compiled validators. Registry-bound `SchemaView` values resolve children in their own registry; the resolver cannot construct IDs or pair raw children with another registry.
- `src/resolve.rs` owns one search and cumulative budget per resolution. It aggregates schema views in completion order; alternative completions reuse that budget.
- `src/pointer.rs` owns RFC 6901 path construction and segment decoding. URI-fragment percent encoding remains part of schema-reference resolution.
- `src/source.rs` owns LF/CRLF/CR boundary mechanics shared by parsing and formatting. `src/parser.rs` owns checked input admission, syntax, and whitespace/comment scanning. Borrowed source becomes owned only after parsing.
- `src/names.rs` owns the shared Unicode key/tag classifier.
- `src/native` owns definition syntax, filesystem/supplied-resource bundle admission, and lowering to the existing in-memory schema engine. It adds no second validator or application-construction path. Default-validation traversal reuses the schema profile's child-position classifier.
- `src/format.rs` owns shared source-preserving layout and trivia mechanics. `src/native/format.rs` renders definition expressions through that printer. The `daml-fmt` binary handles arguments and bounded input/output.
- `src/semantic.rs` owns editable semantic snapshots and bounded canonical data writing. `Schemas` selects source-visible names that retain bound identity; the writer checks the selected output grammar. Names remain borrowed from their input or registry owner.
- `bindings/browser/src` owns JavaScript shape/UTF-16 admission, lossless transport, JavaScript errors, and owned snapshot lifecycle, not language semantics. Input and resolved traversals remain distinct to avoid an intermediate Rust tree and preserve their different provenance requirements.
- `scripts/cargo-notices.py` owns the Cargo notice-tool pin, license-policy translation, template invocation, and temporary-config lifecycle for native and browser packaging. Each packager owns its build, archive, and provenance format.
- The optional `src/lsp` adapter owns in-memory URI/version state and protocol coordinate conversion. It reuses the checked parser and formatter; protocol dependencies and the stdio binary are activated only by the `lsp` feature. It does not read source/schema files or select a schema-folder root.

## Implementation defaults

Keep the library synchronous and safe. The manifest forbids unsafe code and warns about undocumented public APIs. Prefer concrete types, small functions, enums for alternatives, explicit ownership, and structured errors for invalid input. Avoid cloning data merely to silence ownership errors, speculative traits, and an async runtime without an actual I/O concurrency requirement.

Use stable rustfmt and Clippy defaults. Do not enable whole pedantic or restriction lint groups, add a custom formatter style, or introduce a workspace before there is a separate crate boundary. Integration tests should exercise complete documents, ambiguous or unsupported syntax rejection, exact string transformations, source spans, and resource bounds.

The library uses a pinned JSON Schema validator and JSON/Serde support. The isolated fuzz package exercises the real parser, schema ingestion, and resolver with AddressSanitizer. See the [hardening guide](hardening.md) for retained campaign and measurement procedures. Add Miri when an unsafe or FFI boundary needs it. Extra test runners, coverage thresholds, and platform matrices are not baseline requirements.