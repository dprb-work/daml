# python-type-schemas Specification

## Purpose
Derive bounded DAML schema modules from explicitly named Python types, preserving declarative constraints/defaults and rejecting unsupported behavior without requiring Pydantic for base or custom-exporter use.

## Requirements

### Requirement: Explicit Python type bindings produce an ordinary schema module
The Python API SHALL provide `schema_from_types(types, *, exporter=None) -> SchemaModule`, where `types` maps explicit DAML names to Python type expressions. The helper SHALL use existing module admission and SHALL NOT create a second resolver, implicit names, global registry, filesystem discovery or nominal membership.

#### Scenario: A declared worker supplies defaults
- **WHEN** a caller binds `app.Worker` to a Pydantic model or standard dataclass with a required string name and integer retry default of three
- **THEN** the resulting module composes through `Schemas.compose` and resolving `!app.Worker { name: "demo" }` returns name demo and retries three
- **AND** an explicit integer overrides the default while a supplied string retry count fails with the existing resolution error and data path

#### Scenario: Independently exported nested definitions retain local references
- **WHEN** multiple named roots contain nested definitions using local `$defs` references
- **THEN** composition resolves their references in their own resource scopes without treating a nested definition as an automatically registered DAML tag

### Requirement: Pydantic is optional and its exported projection is bounded
The default exporter SHALL use Pydantic v2 TypeAdapter in validation mode with input aliases. The package SHALL expose Pydantic through an optional `types` extra; ordinary import and custom-exporter use SHALL NOT import or require it. Missing optional support SHALL identify the installation extra without masking unrelated import failures.

The supported path SHALL cover JSON-representable records, scalar fields, sequences, string-keyed mappings, nullable/alternative values, literals/enums, declarative constraints and static serializable defaults. It SHALL return completed plain values through existing APIs, not construct Python model instances or adopt Pydantic coercions. DAML's existing required-container completion and mathematical JSON integer semantics remain unchanged.

#### Scenario: Base-only installation uses a custom exporter
- **WHEN** Pydantic is unavailable and a caller supplies an exporter
- **THEN** base import and export/composition/resolution succeed without importing Pydantic

#### Scenario: Unrepresentable Python behavior is rejected
- **WHEN** a model uses a custom validator, default factory, custom serialization/schema override, normalization transform, post-init/custom-init behavior or unsupported core-schema form
- **THEN** the default export fails explicitly with a public exception identifying the affected binding/context rather than omitting the behavior or invoking the factory

#### Scenario: Temporal format is not silently discarded
- **WHEN** an exported schema uses `format` at a schema position, including below definitions or references
- **THEN** export fails explicitly because the current resolver does not assert formats
- **AND** an ordinary property named format or literal default data containing that key is not mistaken for a format keyword

### Requirement: Custom exporters own their schema translation
An explicit exporter SHALL receive each bound Python type expression and return a JSON Schema object or boolean. Its exceptions SHALL propagate without a fallback exporter. The helper SHALL preserve the exported schema and its static defaults, SHALL reject non-JSON representations rather than coerce map keys or non-finite numbers, and SHALL NOT interpret application marker fields or mutate caller-owned exports. Existing native SchemaError/ResolveError behavior SHALL remain intact.

#### Scenario: A generic msgspec struct joins the workflow
- **WHEN** a standalone msgspec struct is supplied with the public `msgspec.json.schema` exporter
- **THEN** the resulting module completes and validates DAML values through the same registry without a core change or application-specific dependency

#### Scenario: Marker conventions remain explicit data
- **WHEN** a custom export contains a literal discriminator property and constant requirement
- **THEN** the requirement remains an ordinary schema constraint, not an implicit DAML tag binding or injected field

#### Scenario: Invalid adapter data is rejected honestly
- **WHEN** a custom export contains non-string object keys, non-finite numbers, unsupported formats or malformed schema content
- **THEN** the appropriate Python export or native admission/composition error is raised instead of returning a weakened contract

### Requirement: The supported workflow is installable, documented and independently checked
The change SHALL provide an example expressing equivalent worker contracts as a Pydantic model and dataclass, deriving modules, completing data and observing meaningful failures. Documentation SHALL identify supported versions, optional installation, unsupported validators/factories/formats, schema composition versus model validation, and the distinction between generated in-memory JSON Schema modules and native definition text. A separate custom exporter demonstration SHALL use a generic type library, not an application project.

#### Scenario: The documented Python workflow runs end to end
- **WHEN** a user installs the supported optional extra and runs the example
- **THEN** the advertised model/dataclass results and invalid-value diagnostics are observed using real dependencies and the native extension
