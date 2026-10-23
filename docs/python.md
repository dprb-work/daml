# Python API

The optional `daml` Python package delegates parsing, formatting, bundle loading,
completion, and validation to the Rust library. It is separate from the native
`daml-fmt` and `daml-lsp` archives. Neither the Rust library nor those executables
requires Python at runtime.

## Checkout and verification

Install [uv](https://docs.astral.sh/uv/getting-started/installation/) and the pinned
Rust toolchain, then run from the repository root:

```sh
./scripts/check-python
```

The command provisions a Python 3.12 project environment, installs locked development
tools, rebuilds the extension, and runs binding Clippy, formatting, Ruff, and Pytest.
`./scripts/check` includes this gate after the Rust checks.

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

`parse` checks syntax only. Unknown tags remain inspectable without importing
anything. `compile` loads reachable definitions; `resolve` completes and validates
values. Neither constructs an application object or invokes a callable.

Tags are separate metadata. `to_python()` deliberately returns plain values;
`tags()` returns literal assertions without `!`, keyed by RFC 6901 pointer. The
root pointer is `""`; a literal key `path/key` appears as `/path~1key`. Resolved
metadata includes tags introduced by defaults. A private alias keeps its literal
spelling in its declaring scope; it does not become a root import or a globally
canonical constructor name.

Parsed documents own their Rust source. Resolved documents own an exported
snapshot and tag metadata, so they survive deletion of the input and registry.
Each export is newly materialized: changing an exported dictionary, list, or tag
map does not change the document or later exports. These exports allocate;
source spelling remains available through `Document.source`.

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

## Entry points

| API | Responsibility |
|---|---|
| `parse(source)` | Syntax-only `Document` |
| `Document.source` | Original UTF-8 text, including comments |
| `Document.to_python()`, `Document.tags()` | Owned value/metadata exports |
| `Document.format(line_width=80)` | Data reflow without completion |
| `format_definition(source, line_width=80)` | Native definition syntax and reflow |
| `Bundle.open(root)` | Required local bundle manifest |
| `Bundle.discover(document_path)` | Nearest ancestor bundle, or `None` |
| `Bundle.compile(document, expected=None)` | Reachable native definitions |
| `Schemas.resolve(document, expected=None)` | Completion and validation |
| `ResolvedDocument.to_python()`, `ResolvedDocument.tags()` | Completed value/metadata exports |
| `SchemaModule.from_json(source)`, `SchemaModule.from_daml(source)` | Low-level interchange envelope |
| `Schemas.compose(modules)` | Explicit interchange composition |

Paths accept strings or `os.PathLike` objects. Filesystem errors preserve
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

Spans are half-open byte ranges, not Python string character indexes. Generated
values have a default origin rather than a fabricated input span. An in-memory
input has no fabricated filename. Rust source chains become Python `__cause__`
chains; filesystem causes use `OSError` subclasses. Materialization errors from
Python's own numeric constructors propagate normally.

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
third-party notices in addition to DAML's license. No PyPI publication,
cross-platform wheel pipeline, consumer construction, editor integration,
YAML/JSON conversion, or Helm integration is implied.
