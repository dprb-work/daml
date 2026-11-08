## Baseline and accepted boundary

Build on `2f5ecbb` (merged MR #26), not the old local primary checkout. Preserve its finite structural recursion, rejection of nonprogressing reference loops, real schema diagnostics, and cumulative pre-validation work bounds. Issues #32–#34 are accepted in order; this change implements #32 only. It does not claim completion of the later editor/source-editing work.

## Commands

```
daml check [--bundle DIR | --module FILE ...] [--expected NAME] [FILE|-]
daml resolve [--bundle DIR | --module FILE ...] [--expected NAME] [--format daml|json] [FILE|-]
daml explain [--bundle DIR | --module FILE ...] [--expected NAME] [--path POINTER] [FILE|-]
```

Omitted FILE reads stdin. Native context discovery uses the actual file path, never cwd. Stdin needs explicit context. All operations require a root assertion or explicit expected name. `--format` is resolve-only and `--path` is explain-only. Repeated singleton options, invalid/missing values, unexpected arguments, and unknown options are usage errors before I/O. `--` terminates option parsing. Help is available at the executable and operation levels.

Exit 0 is success, 1 is configuration/schema/resolution/path/representation failure, and 2 is usage or I/O failure. `check` has empty stdout. `resolve` uses canonical contextual DAML unless `--format json` explicitly selects the tag/kind-losing plain-data projection. `explain` always emits JSON. Serialize successful output only after complete admission; buffer under an explicit output-byte cap before writing so an engine/representation failure emits no partial success data. A broken output stream may necessarily have accepted a prefix and remains exit 2.

## Thin CLI and core explanation owner

The binary owns arguments, bounded file/stdin reads, context selection, output, and exit status. It reuses existing `Bundle`, `SchemaModule`, `Schemas`, semantic writing, names/pointers, and provenance. No new dependency, generic command framework, environment resolver, or release producer is needed.

Core explanation belongs beside `Schemas`/resolution, not in an independent CLI schema walker. Its public entry is `Schemas::explain(document, expected: Option<&str>, pointer: &str) -> Result<Explanation<'_>, ExplainError>`. `Explanation` retains a checked resolved document and exposes `path()`, `node()`, and `constraints()`. A constraint exposes `instance_path`, `location`, and `schema`, retaining registry-owned schema references where practical. Explanation is opt-in; normal resolution must not allocate an explanation trace. The implementation may refine representation within this interface, but cannot weaken admission, omission, alternative-branch semantics, or cumulative bounds.

Prefer authoritative validator evaluation/annotation or actual resolver provenance over reimplementing validation logic. Plain default-completion traces alone are insufficient: validation-only alternatives and ancestor constraints still apply. Keep unresolved alternative semantics represented by their enclosing schema rather than labeling every alternative as unconditionally true. Bound work by actual instance descent and use MR #26 accounting. A raw admitted schema can retain `$ref`; do not materialize recursive graph expansion.

## JSON explanation report

The report has `path`, `value`, `origin`, optional `tagOrigin`, and `constraints`. `value` uses the existing semantic shape: `{kind, value}` for scalars, `items` for lists, `entries: [{key,value}]` for maps, optional `tag: {name,binding}`. Exact numeric payloads are strings; intrinsic scalar kinds remain distinct. Origins use `kind: input` with `span: {start,end}` and optional `source`, `kind: default` with `resource` and `pointer`, or `kind: semantic`. Constraint entries have `instancePath`, `schemaLocation: {resource,pointer}`, and `schema`. Ancestor entries explicitly identify the ancestor path instead of pretending they are node-local facets. The report need not preserve authored formatting or numeric spelling beyond existing semantic guarantees.

## Alternatives and tradeoffs

- Keeping example binaries only cannot meet the supported terminal/CI contract.
- Making JSON the implicit output would silently lose assertions and intrinsic kinds; choose contextual DAML by default and fail when its bound assertions are not source-representable.
- Supporting only native bundles would not exercise MR #26 recursive interchange contracts; repeatable explicit module files use its existing API without adding a second schema format.
- Adding schema discovery/fetching, a cache/service, or a full generic introspection framework adds responsibilities this CLI does not need. Extend the existing owners only for checked explanation.
- Archive inclusion/publication belongs to the active release work. Advertise source builds and coordinate integration explicitly rather than modifying an independently owned producer.

## Evidence and handoff

Independent test ownership covers command outcomes, real value/provenance/constraint behavior, negative paths, exact data, and MR #26 integration. Prompt ownership is coordination, not hostile-write isolation. Run real CLI smoke journeys and the repository gate after integration. Reconcile command/API docs, examine consolidation and simplification, then obtain independent whole-change review. Scope approval is not result acceptance or release authority.
