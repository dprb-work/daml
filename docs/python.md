# Python API

The optional `daml-config` Python distribution (`import daml`) delegates parsing,
formatting, bundle loading, completion, and validation to the Rust library.
Install a verified candidate wheel or build from source. Nothing is published
to PyPI yet. The native `daml-fmt` and `daml-lsp` archives are separate and do not
require Python.

Start with [Configure a Python program with DAML](python-quickstart.md) for a
complete type-derived schema, a configuration file, and visible results.
This page is the API reference.

## Install a candidate wheel

Candidate wheels target standard CPython 3.12 or later on Linux x64, macOS ARM64
and Windows x64. They use `cp312-abi3`, so the same platform wheel works across
compatible standard CPython versions. PyPy and free-threaded builds are not
supported. Linux requires glibc 2.35 or later; it is not a musl/Alpine wheel.

Wheels are qualification artifacts, not published PyPI packages or permanent
release downloads. Obtain the wheel and matching `.sha256` sidecar from a
successful **Binaries** workflow run. Its `daml-config-<Rust target>` artifact
contains the files. With an authenticated GitHub CLI, for example:

```sh
gh run download RUN_ID --repo dprb-work/daml \
  --name daml-config-x86_64-unknown-linux-gnu --dir download
```

Choose the run revision and target deliberately. Verify the wheel against its
sidecar before installing it: use `sha256sum -c` on Linux, `shasum -a 256 -c`
on macOS, or compare `Get-FileHash -Algorithm SHA256` in PowerShell. A checksum
detects corruption, not an unknown publisher's identity.

In a working directory outside the source checkout, create an environment with
[uv](https://docs.astral.sh/uv/getting-started/installation/):

```sh
uv venv --python 3.12
```

Activate it with `. .venv/bin/activate` on Linux/macOS or
`.\.venv\Scripts\Activate.ps1` in PowerShell. Install the exact downloaded wheel;
the following Linux example includes optional Pydantic/dataclass schema support:

```sh
uv pip install --only-binary=:all: './download/daml_config-0.1.0-cp312-abi3-manylinux_2_35_x86_64.whl[types]'
python -c "import daml; print(daml.parse('{ retries: 3 }').to_python())"
```

It prints `{'retries': 3}`. Use the actual wheel filename for your platform.
Omit `[types]` if you only need the base API. A wheel installation needs no Rust
toolchain, source checkout, or native DAML executable. Continue with the
[Python quickstart](python-quickstart.md) using this environment.

The distribution name is **`daml-config`**, but the import remains **`daml`**.
Do not install the unrelated [PyPI `daml` project](https://pypi.org/project/daml/)
in the same environment. A bare `pip install daml` installs that other project;
`pip install daml-config` is not a supported registry route until publication.

## Install from source

Install [Rust through rustup](https://rustup.rs/) and
[uv](https://docs.astral.sh/uv/getting-started/installation/). From a repository
checkout, build the extension into its locked Python 3.12 environment:

```sh
uv sync --project bindings/python --locked --python 3.12 --group dev --reinstall-package daml-config
```

This uses the repository's pinned Rust toolchain and includes development tools.
For installation plus binding Clippy, formatting, Ruff, and behavioral checks,
run `./scripts/check-python`. The aggregate `./scripts/check` includes that gate.

Rerun the gate after changing Rust source. uv's default editable cache does not
track the Rust dependency graph; the gate re-enters maturin with
`--reinstall-package daml-config` so Cargo owns incremental rebuild decisions. See
[uv's cache rules](https://docs.astral.sh/uv/concepts/cache/#dynamic-metadata).
Do not copy a shared library into the environment by hand.

The extension has its own Cargo workspace and lockfile under `bindings/python/`.
Its Cargo manifest owns the package version; maturin supplies that version to
Python metadata. `uv.lock` owns Python development dependencies. This keeps
Python's ABI and build dependencies out of the portable crate, CLI archives, and
fuzz graph. PyO3 0.30.1
and maturin 1.15.0 own the language boundary and wheel packaging; no second parser
or validator is implemented in Python.

## Derive schemas from Python types

The optional `types` extra supports a bounded, explicit path from Pydantic v2
models or standard dataclasses to an in-memory JSON Schema module. Install it
while building the Python extension from this checkout:

```sh
uv sync --project bindings/python --locked --python 3.12 --group dev --extra types --reinstall-package daml-config
```

The minimum Pydantic version is 2.14.2. The base `daml` import and a caller
supplied custom exporter do not require Pydantic. The complete
[Python type example](../examples/python_types.py) derives equivalent contracts
and composes each using the ordinary schema-module API:

```python
from dataclasses import dataclass

import daml
from pydantic import BaseModel

class WorkerModel(BaseModel):
    name: str
    retries: int = 3

@dataclass
class WorkerRecord:
    name: str
    retries: int = 3
module = daml.schema_from_types({"app.Worker": WorkerModel})
schemas = daml.Schemas.compose([module])
resolved = schemas.resolve(
    daml.parse('!app.Worker { name: "demo" }'),
    expected="app.Worker",
)
print(resolved.to_python())
# {'name': 'demo', 'retries': 3}

dataclass_module = daml.schema_from_types({"app.Worker": WorkerRecord})
dataclass_schemas = daml.Schemas.compose([dataclass_module])
print(dataclass_schemas.resolve(
    daml.parse('!app.Worker { name: "demo" }'),
    expected="app.Worker",
).to_python())
```

The mapping keys are explicit DAML assertion names and remain subject to the
DAML source-key grammar. The result is an ordinary in-memory interchange module,
not generated native `.daml` definition text, registered Python classes, or
instantiated model objects. This is a JSON Schema projection, not exact
`model_validate` behavior or strict Python type identity: it makes no promises
about coercion, `extra="ignore"` behavior, or construction of Pydantic instances.
Resolution returns plain values. DAML's required-container completion and
mathematical JSON integer semantics remain unchanged. The default projection
supports string-keyed mappings, but rejects pattern-constrained mapping keys and
constrained model-extra keys: Pydantic's JSON Schema output loses those restrictions.

Models must have resolved annotations before export. If a model refers to a type
defined later, call its Pydantic `model_rebuild()` after defining that type.
The helper rejects incomplete metadata rather than inspecting unresolved names
and risking a weakened schema.
Other type expressions, including named alias targets, must likewise contain
resolved types rather than string forward references.

The default exporter supports declarative JSON-shaped fields, constraints and
static defaults, plus standard-library `date`, `time`, and `datetime` fields
with representable static defaults. Temporal annotations become DAML temporal
values; they do not make the resolver construct Python model instances.
Pydantic `AwareDatetime` requires an explicit offset and `NaiveDatetime`
forbids one. A plain `datetime` permits either. Unsupported comparisons such as
temporal bounds, relative-now rules, and fixed-offset requirements fail rather
than disappear. Default factories are not invoked.

Temporal defaults are recognized from actual Python date/time/datetime objects.
A date-looking string is not promoted to a date. The optional
[temporal example](../examples/python_temporal.py) derives both a Pydantic model
and dataclass, resolves precise typed input, and demonstrates an explicit
standard-library conversion failure.

The exporter still rejects Python behavior it cannot represent declaratively:
custom validators/serializers, schema overrides, normalization, factories,
custom-init/post-init behavior, and unsupported core-schema forms. `format`
assertions supplied as schema positions are rejected because format validation
is disabled; they are not reinterpreted as DAML temporal constructors. Python
type expressions and exporter hooks are trusted caller code and may execute code
during export.

### Work with temporal values

`Document.to_python()` and `ResolvedDocument.to_python()` return immutable
`daml.Date`, `daml.Time`, and `daml.DateTime` objects for intrinsic temporal
values. Construct one from supported ISO text, then use `isoformat()` or `str()`
to get its canonical text. Time and DateTime expose `offset_minutes`; `None`
means no authored offset, while `0` means an explicit UTC offset.

Call `to_date()`, `to_time()`, or `to_datetime()` only when you need a Python
standard-library value. These methods require an exact conversion: unsupported
years, leap-second/end-of-day labels, or nonzero precision beyond microseconds
raise `ValueError`. Extra trailing zero fraction digits can convert exactly;
the DAML value itself retains its original precision either way. DAML temporal
objects are not `datetime` subclasses and are not implicitly converted.

The semantic editing representation uses `kind: "date"`, `"time"`, `"datetime"`,
or `"cron"` with canonical ISO or resolved cron expression text in `value`.
The core Rust `to_json()` projection instead emits canonical strings and loses
intrinsic temporal identity; Python objects require an explicit JSON serialization
choice. Do not use date-looking strings to request typed values; use a temporal
constructor or a real Python temporal object as a static default.

Cron patterns use the same intrinsic lifecycle but expose a different Python API:

```python
pattern = daml.parse('!Cron "0 0 9 LW * *"').to_python()
print(pattern.expression)  # 0 0 9 LW * *
print(str(daml.Cron("@daily")))  # 0 0 0 * * *
```

`daml.Cron` is immutable and validates the
[Spring cron dialect](syntax.md#cron-literals). Its read-only `expression`
contains the resolved pattern. It has no ISO formatting, standard-library
datetime conversion or next-run method. Resolved defaults return the same kind
of object. Equality compares retained resolved expression text, not future
occurrence sets. Plain Python strings do not become Cron values by inference.

There is no standard-library Cron annotation to project. This feature does not
add third-party cron adapters or custom Pydantic hooks to `schema_from_types`;
use native `!Cron`, explicit schema metadata or the existing custom-exporter
boundary. The [runnable cron example](../examples/python_cron.py) resolves a
native bundle and edits its typed patterns.


An explicit `exporter(type) -> JSON Schema object or boolean` owns its translation
conventions and errors; exceptions propagate without fallback. For example, a
generic `msgspec` exporter is shown separately below. A custom schema's marker
property remains a schema property; it does not become an implicit DAML tag.
See the [interchange profile](schemas.md) for the limits shared with native
schema admission and resolution.

### Generic custom exporter with msgspec

Install `msgspec` separately if you want this optional demonstration; it is not
required by the Python `types` extra:

```sh
uv pip install --python bindings/python/.venv/bin/python msgspec
```

```python
import msgspec
import daml

class RetryConfig(msgspec.Struct):
    retries: int = 3

module = daml.schema_from_types(
    {"app.RetryConfig": RetryConfig},
    exporter=lambda type_: msgspec.json.schema(type_),
)
schemas = daml.Schemas.compose([module])
resolved = schemas.resolve(
    daml.parse("!app.RetryConfig {}"),
    expected="app.RetryConfig",
)
print(resolved.to_python())
```

This illustrates only the generic exporter boundary: DAML does not interpret
library-specific markers or rewrite exporter output.

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
completes and validates supplied data against the compiled schemas.

Tags are separate metadata. `to_python()` deliberately returns plain values;
`tags()` returns literal assertions without `!`, keyed by RFC 6901 pointer. The
root pointer is `""`; a literal key `path/key` appears as `/path~1key`. Resolved
metadata includes tags introduced by defaults. A private alias keeps its literal
spelling in its declaring scope; it does not become a root import or a globally
visible definition name.

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
`string`, `date`, `time`, `datetime`, `cron`, `list`, or `map`. Numbers carry an
exact JSON numeric string in `value`; temporal nodes carry canonical ISO text
or resolved cron expression text. Lists use
`items`; maps use `entries` of `{"key": ..., "value": ...}` records.
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

Use `schemas.validate_semantic(editable, expected=...)` to check constraints
without completion. It accepts the same semantic tree and embedded tag/binding
assertions as `resolve_semantic`, returns `None` on success, and raises the same
admission or constraint errors on failure. It does not insert defaults, construct
missing containers, invoke callbacks, or mutate the tree. Missing required fields
remain missing and fail validation. Nested explicit assertions are checked even
under open fields.

Validation accepts literal JSON keys such as `$ref` within the normal key byte
budget. Success does not promise DAML serialization: `resolve_semantic` and
writing still reject keys outside DAML's source grammar. Use constraint-only
validation when omission must remain omission.

Both JSON-module and native-definition contracts may recurse through instance
children. Finite nested values retain their field constraints and defaults;
non-descending reference loops are rejected. Completion remains bounded:
unending required-container or default expansion raises `LimitExceeded` instead
of silently omitting a required value or inventing a leaf.

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
| `Schemas.validate_semantic(value, expected=None)` | Validate supplied semantic data/assertions without completion or mutation |
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

## Prepare wheels

Use the repository-owned producer, not a bare `maturin build`, when preparing
redistributable candidates:

```sh
python scripts/package-python.py --out dist/python
```

The build needs the pinned Rust toolchain, maturin 1.15.0 and cargo-about 0.9.2.
The producer includes the canonical MIT license, source inventory, generated
dependency notices and build provenance. It writes the wheel and a matching
checksum without uploading either. See
[Python wheel preparation](development/distribution.md#python-wheel-candidates)
for prerequisites, the platform matrix and installed-artifact verification.

Source builds still use `uv sync` above. No Python sdist, wheel release asset or
PyPI upload is implied by a candidate build.
