## Why

Configuration authors should use a schema derived from the Python types a program already declares, rather than maintain the same field model twice. The first supported path uses Pydantic v2 and standard dataclasses, while other Python representations can supply the same schema-export boundary.

## What Changes

- Add `daml.schema_from_types(types, *, exporter=None) -> SchemaModule`: callers explicitly bind DAML names to Python types; the result composes through the existing native schema API.
- Use Pydantic TypeAdapter for the default export path, including standard dataclasses, with an optional `types` dependency extra. Base DAML import and explicit custom exporters do not require Pydantic.
- Reject Python validation/default behavior that cannot be represented by the supported JSON Schema projection, and reject format assertions because the existing engine treats formats as annotations.
- Accept caller-owned JSON Schema exporters without a registry or application-specific imports; demonstrate the public msgspec exporter with standalone structs.
- Add regression coverage and a readable end-to-end Python example. Update documentation to describe the newly supported bounded path rather than claiming no type-schema generation exists.
- No core parser/resolver changes, object construction framework, native-definition text generator, file discovery, CLI, temporal/cron feature, registry publication, or cross-repository integration.

## Capabilities

### New Capabilities

- `python-type-schemas`: bounded Python type-to-schema projection with optional Pydantic support and an explicit custom exporter.

### Modified Capabilities

None. Existing schema admission, composition, completion, validation, exact values, names and errors remain authoritative.

## Impact

Python package source, optional/development dependencies and lockfile, Python tests, documentation and examples. No changes to Rust/binding implementation, existing tests or verification selection without the test owner's explicit disposition.

Scope authority: the user selected Python, Pydantic and dataclasses, accepted the generic exporter approach, and instructed implementation with “yes, lets go.” This proposal records that accepted boundary; detailed API naming and the supported projection are implementation decisions within it. Publication of a package or cross-project migration is not authorized by this change.
