## Context

The existing Python boundary exposes SchemaModule.from_json and Schemas.compose. Rust already assigns each type a resource identity, scopes local references, bounds admitted modules and owns resolution. Reuse that owner rather than introduce an intermediate type system or plugin framework.

Pydantic TypeAdapter exports both BaseModel and standard dataclasses. A probe on Pydantic 2.14.2 showed that factories and callback validators disappear from ordinary JSON Schema while remaining explicit in TypeAdapter.core_schema. The adapter must reject those shapes before schema generation. The native resolver deliberately disables format assertions, so generated formats must not be admitted as if enforced.

## Goals / Non-Goals

Goals: one explicit Python helper, an optional supported Pydantic path, a caller-owned exporter path, declarative constraints/defaults, honest limitations and generic conformance examples.

Non-goals: arbitrary reflection, model construction/coercion, application registries, automatic import/discovery, native DSL file generation, CLI/editor work, date/time/cron support, package publication or application migration.

## Decisions

- Public helper: `daml.schema_from_types(types, *, exporter=None) -> SchemaModule`. A private Python implementation module is re-exported from the existing initializer with annotations; native extension stubs do not pretend to own this Python function.
- Default exporter loads Pydantic lazily only when needed. Add optional extra `types` with `pydantic>=2.14.2,<3`; development dependencies include that requirement and pinned msgspec 0.22.0 for real custom-exporter regression coverage.
- Reuse TypeAdapter's validation-mode, alias-aware schema generator. Inspect supported core-schema nodes for unrepresentable callbacks/factories/transforms and reject unknown forms rather than invent a reflection engine. Walk schema positions, not arbitrary default/metadata dictionaries. Treat Python type/schema hooks as trusted caller code, not configuration-driven execution.
- The custom exporter callable is the extension point. No abstract provider class, registration mechanism or special msgspec/consumer logic. A custom exporter owns what it translates; native schema errors are not swallowed or replaced.
- Preserve each root schema's local reference scope through the existing module envelope. Root names are explicit; nested `$defs` are not implicitly registered tags.
- Serialize only JSON-compatible exports with string keys and finite numbers. Do not mutate exports, execute default factories, coerce values, apply model validators, overwrite resource identities or precompose modules solely to validate them twice.
- Reject format keywords in actual schema positions on both routes. Do not turn annotations into global engine assertions as part of this feature. Temporal support remains its own accepted contract later.
- Public ValueError/TypeError describe export problems with binding/context; native SchemaError and ResolveError continue describing admission/composition/value failures. Unexpected exporter exceptions preserve their original cause, with no fallback.

## Risks / Trade-offs

- JSON Schema cannot express all Pydantic runtime semantics. This is a declarative JSON contract projection, not a promise of exact model_validate equivalence, strict Python identity checks or serialization round trips.
- A conservative supported core-schema subset may reject legitimate advanced models. Explicit rejection and caller-owned exporters are preferable to silently discarded constraints.
- Required container completion and numeric lexeme behavior follow DAML, not raw Pydantic validation. Examples and tests must make that boundary visible.
- Adding Pydantic to the core dependency set would burden unrelated users; the optional import and base-only scenario are required.
- Generic msgspec export does not imply every custom subclass's marker, callable or default policy is automatically supported. Those remain with the custom exporter.

## Migration Plan

Additive Python API and optional extra; no migration of existing native APIs or schema modules. Update current documentation statements that all definitions must be handwritten once the supported adapter works. No compatibility shim or provider deployment is needed.
