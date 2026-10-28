# Python API

The optional `daml` Python package delegates parsing, formatting, bundle loading,
completion, and validation to the Rust library. Install it from a source checkout;
no PyPI package or release wheel is provided. It is separate from the native
`daml-fmt` and `daml-lsp` archives. Neither the Rust library nor those executables
requires Python at runtime.

## Install from source

Install [Rust through rustup](https://rustup.rs/) and
[uv](https://docs.astral.sh/uv/getting-started/installation/). From a repository
checkout, build the extension into its locked Python 3.12 environment:

```sh
uv sync --project bindings/python --locked --python 3.12 --group dev --reinstall-package daml
```

This uses the repository's pinned Rust toolchain and includes development tools.
For installation plus binding Clippy, formatting, Ruff, and behavioral checks,
run `./scripts/check-python`. The aggregate `./scripts/check` includes that gate.

Rerun the gate after changing Rust source. uv's default editable cache does not
track the Rust dependency graph; the gate re-enters maturin with
`--reinstall-package daml` so Cargo owns incremental rebuild decisions. See
[uv's cache rules](https://docs.astral.sh/uv/concepts/cache/#dynamic-metadata).
Do not copy a shared library into the environment by hand.

The extension has its own Cargo workspace and lockfile under `bindings/python/`.
Its Cargo manifest owns the package version; maturin supplies that version to
Python metadata. `uv.lock` owns Python development dependencies. This keeps
Python's ABI and build dependencies out of the portable crate, CLI archives, and
fuzz graph. PyO3 0.30.1
and maturin 1.15.0 own the language boundary and wheel packaging; no second parser
or validator is implemented in Python.

## Parse, complete, and inspect

After building, start the project interpreter:

```sh
uv run --project bindings/python --locked --python 3.12 python
```

```python
from pathlib import Path
import daml

root = Path("examples/native")
source = (root / "data/omitted.daml").read_text(encoding="utf-8")
document = daml.parse(source)
bundle = daml.Bundle.open(root)
schemas = bundle.compile(document)
resolved = schemas.resolve(document)

value = resolved.to_python()
assert value["adapter"]["timeout"] == 45
assert resolved.tags()["/adapter"] == "provider.Http"
```

`parse` checks syntax only. `compile` loads reachable definitions; `resolve`
completes and validates values. These APIs operate on configuration data and
schemas; they do not construct application-specific objects or invoke callables.

Tags are separate metadata. `to_python()` deliberately returns plain values;
`tags()` returns literal assertions without `!`, keyed by RFC 6901 pointer. The
root pointer is `""`; a literal key `path/key` appears as `/path~1key`. Resolved
metadata includes tags introduced by defaults. A private alias keeps its literal
spelling in its declaring scope; it does not become a root import or a globally
canonical constructor name.

Parsed documents own their Rust source. Resolved documents own one semantic
snapshot, so they survive deletion of the input and registry. Each export is
newly materialized: changing an exported dictionary, list, or tag map does not
change the document or later exports. These exports allocate; source spelling
remains available through `Document.source`.

### Explicit expected root

Supply the expected name at both phases:

```python
document = daml.parse('{ name: "worker" }')
schemas = bundle.compile(document, expected="app.Config")
resolved = schemas.resolve(document, expected="app.Config")
```

Compilation loads the requested definition; it does not make that root policy
sticky. Resolving an untagged document without `expected` does not apply that
contract. An explicit tag and an expected contract both remain in force.

Pass `additional_types=("self.Target", ...)` to `Bundle.compile` when a consumer
needs other entry-visible definitions after compilation. This loads their native
constraints/defaults without applying them to the document's root. Private
dependency aliases do not become entry-visible names. Name inputs are bounded,
including duplicates; unknown caller-supplied names have no fabricated source
span, while failures inside loaded definitions retain their actual spans.

`Bundle.from_sources(root, sources)` accepts a string root and an owned snapshot
of a `dict[str, str]` resource table, using the same compiler as `Bundle.open`.
It performs bounded UTF-8 admission before copying resources and never falls
back to filesystem or network lookup. Changing the supplied dictionary later
does not change the bundle. Logical paths and bundle boundaries follow the
[native resource contract](native-authoring.md#files-and-bundle-roots).

### Exact numbers

No number passes through a Python float:

| DAML spelling | Python value |
|---|---|
| `123456789012345678901234567890` | Arbitrary-precision `int` |
| `1.2300e-40` | `Decimal("1.2300e-40")` |
| `1e2` | `Decimal("1e2")` |

Fractional or exponent spelling selects `decimal.Decimal`; integer spelling
selects `int`. Assertions validate rather than cast. Exported semantic values do
not promise to preserve every original numeric spelling. Standard Python integer
string limits still apply during materialization; DAML does not disable them
process-wide. `Decimal` also needs an explicit policy when a consumer later emits
JSON/YAML; it is not silently converted to float.

### Edit and re-admit exact values

`Document.value()` and `ResolvedDocument.value()` return editable semantic trees,
not ordinary JSON-shaped values. Each node has `kind`: `null`, `bool`, `number`,
`string`, `list`, or `map`. Numbers carry an exact JSON numeric string in `value`;
lists use `items`; maps use `entries` of `{"key": ..., "value": ...}` records.
An optional `tag` retains the assertion name and, after resolution, its opaque
binding identity. Numeric strings in this representation are number nodes, not
string data.

```python
document = daml.parse("{ rate: 0.10000000000000001 }")
editable = document.value()
editable["entries"][0]["value"]["value"] = "0.10000000000000002"
source = daml.write(editable)
assert daml.parse(source).to_python()["rate"] == Decimal("0.10000000000000002")
```

With a compiled registry, use `schemas.resolve_semantic(editable, expected=...)`
to complete and validate edited values through the same resolver. The input is
not trusted because an earlier version came from `resolve`: changed values,
assertions and expected-root constraints are checked again.

Use `schemas.write(editable)` for contextual source writing. It preserves bound
assertion meaning and uses an equivalent visible spelling only when one exists.
It raises `SemanticError(kind="UnrepresentableTag", ...)` rather than printing a
private assertion through a colliding entry alias. Module-level `write` has no
registry and cannot invent binding-preserving qualified spellings. Do not strip
binding metadata to make a write succeed.

Parsed assertions have literal names without resolved bindings. Resolved source
and default assertions may carry opaque binding identifiers. They are not Python
names, import paths or stable presentation strings. A supplied binding must
identify its canonical contract in the receiving registry; an unknown identifier
or a local alias wrapper is not retried using its literal tag name. Pure aliases
share canonical meaning; constrained aliases remain distinct.

Semantic inputs have no original byte span. Only defaults inserted by the new
admission have default provenance. Exporting and re-admitting a whole completed
tree is a new programmatic input; it does not preserve the first call's per-node
input/default origins or an application's original references and omission
states. Retain authored source separately when those states matter.

The bridge rejects malformed node shapes, unknown fields, duplicate map keys,
invalid numeric tokens, invalid Unicode and cyclic Python graphs before writing
or resolution. Shared acyclic values are allowed. It bounds depth, nodes, scalar
bytes and aggregate bytes; the core also enforces its normal exact-number and
resolution limits. No value passes through a Python float.
Optional expected-type names on semantic and staged entry points use the same
bounded UTF-8 admission and cumulative invocation budget as the value. Oversized
names fail before registry lookup or construction of a large lookup diagnostic.

### Stage consumer transformations before strict admission

Use `schemas.resolve_staged(document, stage, expected=None, initial_edits=None)`
or `resolve_semantic_staged(value, stage, ...)` when defaults must be visible
before a consumer transformation. The [core staged contract](schemas.md#staged-admission)
owns edit semantics, assertion conservation, candidate selection and bounds.
DAML does not recognize an application's reference or constructor syntax.

The stage object provides `defer(summary, path) -> bool` and
`materialize(value, copies, round)`. Optional `number_class(lexeme)` returns a
non-bool integer from 0 through 4,294,967,295. Summaries use semantic scalar
`kind`/`value` and optional own-node `tag`; containers have `kind` and `length`.
A map root also has `fields` records with `key` and shallow `summary`. Field
containers omit descendants, and fields never borrow an ancestor's tags.

`materialize` receives fresh semantic data and Copy-history lists. Returning None
means Ready, bytes means Ready with a meaning key, a list means edits, and a
returned `BaseException` rejects that candidate. A raised exception is fatal and
propagates unchanged. These calls are synchronous; coroutine or other invalid
return shapes are rejected.

Edit dictionaries are `{"op": "replace", "path": ..., "value": ...}`,
`{"op": "copy", "path": ..., "from": ...}`, or
`{"op": "expect", "path": ..., "name": ...}`. Initial edits default to none;
explicit lists/tuples are admitted once and cannot contain Copy. Preload named
expectations with `Bundle.compile(..., additional_types=...)`.

The checked result retains the existing owned value/tag exports and adds
`meaning() -> bytes | None`. The key belongs to the winning checked candidate.
Mutating callback data, returned exports or an edit list later does not mutate
the result. Copy-operation history alone does not promise Python object sharing;
the application owns construction and freshness.

## Entry points

| API | Responsibility |
|---|---|
| `parse(source)` | Syntax-only `Document` |
| `Document.source` | Original UTF-8 text, including comments |
| `Document.to_python()`, `Document.tags()` | Owned value/metadata exports |
| `Document.value()` | Editable exact semantic tree with literal assertions |
| `Document.format(line_width=80)` | Data reflow without completion |
| `format_definition(source, line_width=80)` | Native definition syntax and reflow |
| `Bundle.open(root)` | Required local bundle manifest |
| `Bundle.discover(document_path)` | Nearest ancestor bundle, or `None` |
| `Bundle.from_sources(root, sources)` | Explicit owned UTF-8 resource table; no filesystem fallback |
| `Bundle.compile(document, expected=None, additional_types=None)` | Reachable and explicitly loaded native definitions |
| `Schemas.resolve(document, expected=None)` | Completion and validation |
| `Schemas.resolve_semantic(value, expected=None)` | Complete and validate edited semantic values |
| `Schemas.resolve_staged(document, stage, expected=None, initial_edits=None)` | Shared staged completion followed by strict admission |
| `Schemas.resolve_semantic_staged(value, stage, expected=None, initial_edits=None)` | Same stage for editable semantic input |
| `Schemas.write(value, line_width=80)` | Binding-preserving contextual source writing |
| `write(value, line_width=80)` | Unqualified semantic source writing |
| `ResolvedDocument.to_python()`, `ResolvedDocument.tags()` | Completed value/metadata exports |
| `ResolvedDocument.value()` | Fresh editable snapshot retaining opaque assertion bindings |
| `ResolvedDocument.meaning()` | Selected staged meaning bytes, or `None` |
| `SchemaModule.from_json(source)`, `SchemaModule.from_daml(source)` | Low-level interchange envelope |
| `Schemas.compose(modules)` | Explicit interchange composition |

Filesystem paths accept strings or `os.PathLike` objects. Filesystem errors preserve
lossless OS path strings, including surrogate escapes on Unix. Formatting width
must be positive. Native definitions and low-level interchange remain separate
APIs; the [schema guide](schemas.md) owns the bounded Draft 2020-12 profile.
The canonical native stub is `bindings/python/python/daml/daml.pyi`. The public
package re-exports the native API through its initializer and includes a
`py.typed` marker. Maturin's standard mixed-package layout keeps these files
together in both wheels and source-built installations. Composition accepts
iterables of `SchemaModule`, including one-shot iterators and generators.

## Errors

The public exception classes derive from `ValueError`:

| Exception | Structured fields |
|---|---|
| `ParseError` | `kind`, UTF-8 byte `span`, `path=None` |
| `NativeError` | `kind`, optional file `path`, optional byte `span`, optional `reference` |
| `SchemaError` | `kind`, optional `schema=(resource, pointer)` |
| `ResolveError` | `kind`, instance `path`, optional input `span`, optional `schema`, optional default `origin=(resource, pointer)` |
| `SemanticError` | `kind`, RFC 6901 `pointer` |

Spans are half-open byte ranges, not Python string character indexes. Defaults
inserted by resolution have a default origin rather than a fabricated input
span. Supplied semantic values report their instance pointer, with `span=None`
and `origin=None`. An in-memory input has no fabricated filename. Rust source
chains become Python `__cause__` chains; filesystem causes use `OSError`
subclasses. Materialization errors from Python's own numeric constructors
propagate normally.

## Local packages

Build a wheel and a self-contained source archive from the repository root:

```sh
uv run --project bindings/python --locked --python 3.12 maturin build \
  --manifest-path bindings/python/Cargo.toml --release --locked \
  --interpreter bindings/python/.venv/bin/python --out .scratch/python-dist
uv run --project bindings/python --locked --python 3.12 maturin sdist \
  --manifest-path bindings/python/Cargo.toml --out .scratch/python-dist
```

Maturin includes the Rust path dependency and adjusts the source archive's build
entrypoint. The Python project and Cargo package metadata declare MIT. The
canonical root `LICENSE` is not included by an external-path metadata reference:
Cargo rejects a license-file path outside this workspace. Python wheel and sdist
license-text and third-party-notice inclusion have not been verified, so those
artifacts are not part of the current release route. Native compilation from
source still needs Rust. A matching wheel needs only a compatible Python
interpreter and platform libraries.

The ABI floor is CPython 3.12 (`cp312-abi3`). Local Linux checks do not establish
Windows, macOS, PyPy, or free-threaded compatibility. No Python wheel or sdist
release assets are currently provided; any future redistribution needs generated
third-party notices in addition to DAML's MIT license.
