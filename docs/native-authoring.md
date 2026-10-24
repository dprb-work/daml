# Native definitions and bundles

This contract describes native authoring, separate from the JSON-compatible
[interchange module](schemas.md). The [implementation plan](../PLAN.md#next-sequence)
records implementation and verification status. Native compilation produces
in-memory JSON Schema constraints and DAML completion metadata, not generated
JSON Schema files or application objects.

## Files and bundle roots

A bundle is a directory containing a required `daml.bundle` manifest and
one-definition `.daml` files. A definition's name follows its path. The manifest
is a flat map of local aliases to directory paths:

```daml
{
  app: "./app"
  http: "./http"
}
```

An empty manifest is `{}`. Relative paths are anchored to the declaring
manifest's directory. Filesystem bundles permit absolute local paths. URI,
network, package, and environment-variable resolution are not implied by a path string.

`!app.network.Host` selects binding `app`, then `network/Host.daml` beneath that
root. Only dots separate tag segments; slashes are not alternative separators.
Directories and filename stems used by a tag must obey the segment grammar.
The `.daml` suffix is added by the loader. A dot in the physical bundle-root
name is harmless; a dot within a definition stem is not an escaped segment.

Every loaded bundle has its own bindings. Imports are not repeated in individual
files and do not leak through dependencies. `!self.Host` selects `Host.daml`
from the current bundle root, not from the referring file's directory. `self`
cannot be rebound.

A nested `daml.bundle` establishes a boundary. Import that directory under an
alias; do not reach its definitions by walking through it from an outer root.
Boundary diagnostics must name the nested bundle and recommend an explicit
binding. No silent context switching or import-table merging occurs.

The loader canonicalizes bundle roots. Aliases to the same root and relative
definition path share a definition; separate copies are separate bundles.
Definition paths must not escape their selected canonical root or bypass bundle
boundaries through filesystem links. This governs ownership, not nominal
compatibility. A referenced definition remains a structural contract.

Only referenced definitions are loaded: selected root types, explicit data tags,
constraint references, and references/tags in declared defaults. Unused invalid
files are not scanned. Manifest shape is admitted when the bundle is loaded;
unreferenced dependency directories are not opened just because their aliases
are declared. Bundle-import cycles are allowed; definition-reference cycles
remain rejected by the bounded validation profile.

Tooling discovers the nearest ancestor manifest starting at a document's
directory, unless a caller explicitly selects a context. Cwd, Git roots, and
editor workspace roots are not discovery rules. Untitled/stdin documents need
an explicit context for bundle resolution. A root tag or explicit caller chooses
the expected root definition; there are no filename/glob association tables.

`Bundle::from_sources(root, sources)` selects the same compiler with an explicit
in-memory resource table instead of filesystem discovery. Resource keys are
normalized slash-relative IDs; root is `.` or a relative directory. Manifest
locators such as `.`, `./dep/`, and confined parent references resolve within
that supplied namespace; absolute paths and namespace escapes are rejected.
Referenced imports remain private and lazy, and nested manifests still establish
bundle boundaries. The [browser SDK](../bindings/browser/README.md#resource-table-and-bounds)
documents table admission and its static browser binding.

## Definition expressions

Constraints precede `=`; default data follows it:

```daml
closed !Object {
  endpoint: !String
  retries: !Int
} = {
  endpoint: "localhost"
  retries: 3
}
```

Members are required unless marked `?`, and every declaration uses
`key: constraint`. A literal constraint such as `mode: "http"` accepts only
that value and supplies no default. Inferred declarations such as
`mode = "http"` are not supported.

Scalar field defaults belong in the containing record's preset, as in the
example above, not in declarations such as `retries: !Int = 3`. Grouping or
alternatives do not make an inline scalar default valid. Root scalar
definitions can still supply defaults, for example `!Int = 3`.

Nested container presets stay attached to their own definitions:
`settings: !Object { retries: !Int } = { retries: 3 }`. Constructor metadata
constraints such as `List.items` and `Map.values` retain their own presets.
Removing a nested container preset with `= unset` requires a container
constraint; reference-dependent admission happens during compilation, not
filesystem access by the syntax parser. An unconstrained or scalar-capable
field cannot use inline `= unset`. Optional members cannot carry an explicit
default.

The definition body and default body are different grammar positions. Defaults
are values, so object members after `=` use `key: value`, never another layer
of `key = value` declarations. Constraints select Int versus Number explicitly;
numeric spelling is preserved rather than used to infer a field's type.

| Constructor | Constraint |
|---|---|
| `!String` | String value; optional `min`, `max`, and typed `pattern` facets |
| `!Bool` | Boolean value |
| `!Int` | Mathematically integral JSON-style number; inclusive/exclusive bounds and exact positive `multipleOf` |
| `!Number` | Any valid JSON-style number with inclusive/exclusive bounds and exact positive `multipleOf` |
| `!Object { ... }` | Named record members; open unless prefixed by `closed` |
| `!List { items: ... }` | Homogeneous list; optional `min`, `max`, and `unique` facets |
| `!Tuple [ ... ]` | Exact-length positional sequence; one constraint per position, with no length or uniqueness facets |
| `!Map { values: ... }` | Arbitrary-key mapping; optional `min` and `max` property-count facets |
| `!Any` | Any supplied value; does not disable explicit assertions |
| `!Never` | No value |

Int/Number `min` and `max` are inclusive numeric bounds; `exclusiveMin` and
`exclusiveMax` exclude their boundary. A definition cannot supply both lower
forms or both upper forms. `multipleOf` requires a positive numeric literal and
uses exact decimal arithmetic:

```daml
probability: !Number {
  exclusiveMin: 0
  exclusiveMax: 1
  multipleOf: 0.05
}
```

String `min` and `max` count Unicode scalar values. List bounds count items; Map
bounds count properties. Count facets require non-negative integer literals.
`unique: true` uses JSON data equality, so tags and provenance do not make equal
values unique; `unique: false` is rejected as a no-op. Constraints validate the
completed value, including fields inserted from defaults.

A record can acquire property-count bounds by intersection without opening a
closed record:

```daml
closed !Object {
  primary?: !String
  secondary?: !String
} & !Map {
  min: 1
  max: 1
}
```

String patterns use a typed, raw Regex literal:

```daml
identifier: !String {
  min: 3
  max: 32
  pattern: !Regex "[\p{Alphabetic}_][\p{Alphabetic}\p{Number}_-]*"
}
```

The quotes delimit the Regex but do not invoke DAML string escapes; the compiler
passes backslashes through to Rust `regex`. Regex literals are single-line,
compile during schema admission under the same 1 MiB program/DFA limits as
interchange patterns, and match the complete string. Rust syntax and Unicode
classes apply directly; look-around and backreferences are unsupported. A Regex
literal is valid only as `String.pattern`, not as an instance-data type.

Unknown constructor metadata is rejected rather than ignored. Broader JSON
Schema constraints remain available through interchange; the
[Draft 2020-12 baseline audit](json-schema-2020-12-audit.md) separates backend
support from native expressibility.

Unqualified names are native built-ins. Loaded definitions use a bundle prefix.
Low-level explicitly composed interchange bindings remain a separate API.

## Composition, records, and positions

`A | B` accepts either constraint inclusively. `A & B` requires both. `&` binds
more tightly than `|`; parentheses group expressions. A trailing default belongs
to the complete expression:

```daml
!Object {
  timeout: !shared.Timeout & !Int {
    max: 60
  }
} = {
  timeout: 10
}
```

Grouping does not change default replacement/removal or make an optional default
declaration valid. A bare tagged default may end at `)` without an explicit `{}`.
Grouped triple-string defaults retain their standalone closing-delimiter line
when reformatted.

Referenced constraints always remain in force. Narrowing cannot erase a bound,
weaken requiredness, or reopen a closed base. `closed !base.Record { ... }`
closes the complete referenced field set, including inherited members. An open
base may be extended and the resulting definition closed; a closed base cannot
be extended with fields it forbids.

`port?: !Never` forbids a particular field without closing the rest of a record.
Explicit null is still a value and does not satisfy Never.

`!Tuple [ ... ]` declares exact-length positional constraints. `!List` instead
applies its `items` constraint to every element and may specify length and
uniqueness facets:

```daml
range: !Tuple [
  !Int
  !Int
] = [
  1
  10
]
```

Each tuple position recursively contains a constraint. Object patterns remain
open unless closed. Missing or additional positions are invalid; completion
does not create positions. Equal values in different positions are allowed.
`!Tuple []` accepts only an empty sequence. Tuple takes a bracket body, not a
metadata object, and has no `min`, `max`, or `unique` facets.

Direct defaults and `= unset` are not permitted on individual tuple positions,
including through grouping or alternatives. Referenced record presets and
nested container presets still complete existing elements. Supply the whole
tuple's default after its complete definition. A derived definition replaces it with
`!self.Base = [2]` or removes it with `!self.Base = unset`; neither operation
removes the inherited positional constraints.

Bare arrays are ordinary sequence data, not positional definitions. Arrays
after `=` remain ordinary default data. To narrow a referenced definition with
positional constraints, write `!self.Base & !Tuple [ ... ]`; reference-plus-bare-
brackets is not an alternative tuple spelling.

## Defaults and completion

Definition overrides and configuration completion have different rules:

- A replacement default component replaces its inherited default completely,
  including defaults on referenced record members. `= unset` removes that
  component; neither operation removes structural requirements or unrelated
  member defaults. The replacement applies before configuration prefill, even
  when a partial object is supplied.
- Untagged configuration objects are prefilled from object presets. Supplied
  values win, and missing members receive applicable member defaults.
- Any explicitly tagged supplied object replaces that field's preset, even
  when it resolves to the same definition. Its selected definition's own
  defaults still apply. The rule is tag presence, not identity comparison.
- Supplied lists/scalars replace their defaults. List defaults are never zipped
  or concatenated with supplied lists.
- Required containers use explicit defaults when present; otherwise records/maps
  start from `{}` and lists from `[]`, then complete and validate.
- Optional absent members remain absent, including references to definitions
  with root presets. Supplied optional objects still receive applicable presets
  and member defaults. Any has no uniquely constructible shape. Primitive
  values, implementation tags, map entries, and list positions are not guessed.
  Explicit null never means absence.

A list's item default prefills existing elements but creates no elements:

```daml
workers: !List {
  items: !Object {
    name: !String
    retries: !Int
  } = {
    retries: 3
  }
} = [
  {
    name: "primary"
  }
]
```

A supplied list containing only a worker named `secondary` replaces the default
list. That worker receives retries 3, not the default worker's name. A supplied
empty list remains empty. An element lacking its required name fails.

Defaults may be partial. Full validation follows completion. Admission applies
available defaults and checks whether a partial preset can satisfy its concrete
constraints; it does not require a unique finished configuration. Missing
required names and lower property-count bounds are deferred because later
configuration can add fields. Upper property-count bounds and constraints on
existing values still apply, including fields inserted from available defaults.
This does not require a general satisfiability solver. Conflicting valid defaults
fail only when missing configuration data needs competing fallbacks; explicit
data or an explicit replacement default can resolve that conflict.

Union completion never chooses by alternative order. No valid completion is an
error; one semantic result is accepted; different valid completed results are
ambiguous. An ambiguous omitted union does not itself invalidate its definition.

Default tags resolve in the declaring bundle's scope, even when that alias is
private to the bundle. The literal tag spelling is retained alongside its
resolved binding. Explicit configuration tags resolve in the entry context.
Unknown explicit tags fail rather than falling back to a preset.

## Names and diagnostics

Ordinary keys are unquoted strings: Unicode letters/numbers, subsequent combining
marks, and `_ - . /`. They may start with digits; dots/slashes never imply lookup
or loading. Tag segments and aliases use the same word characters but only
`_ -` punctuation, with dots as separators. First characters cannot be combining
marks. No normalization or case folding is performed.

Definition/source failures retain filename, source range where applicable, and
reference context. Runtime failures retain data paths and original/default
origins. Generated values never receive fabricated input spans. Existing parser,
composition, cumulative resolution, definition/reference traversal, and compiler
materialization bounds remain active. Schema/default/preset copies are charged
before allocation; reuse cannot amplify a source-sized default without limit.

## Acceptance examples

`examples/native/` is a complete entry context with separate application,
network-contract, and provider bundles. Its data cases cover:

| Case | Expected result |
|---|---|
| `patched.daml` | Untagged partial object retains the contextual preset and private provider assertion |
| `omitted.daml` | Root presets, private tagged provider default, item completion, empty required map |
| `supplied.daml` | Tagged field-preset replacement, provider defaults, list replacement, Unicode/dotted/slashed/numeric keys |
| `untagged-nested.daml` | Nested assertions apply without inventing a whole-document root contract |
| `missing-worker-name.daml` | Missing required element member; no index merging |
| `empty-range.daml` | Exact-length tuple validation; no position synthesis |
| `forbidden-field.daml` | Never rejects a present field |
| `inherited-invalid-default.daml` | Narrowing excludes an inherited fallback; definition compilation fails |
| `private-alias.daml` | Entry cannot use a dependency's private alias |
| `boundary-crossing.daml` | Explicit import required at a nested bundle boundary |

The runnable native example loads this entry context and resolves a data file:

```sh
cargo run --locked --example native -- examples/native examples/native/data/omitted.daml
```

The [representative consumer conversion](../examples/olaf/README.md) exercises
the same loader without importing its source application's runtime. The
[Python API](python.md) wraps this path without adding another grammar or
completion engine.

Formatting and syntax-only LSP diagnostics share the native/data syntax reader;
formatting does not resolve definitions, fetch bundles, or apply defaults.
