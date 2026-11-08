## MODIFIED Requirements

### Requirement: Pydantic is optional and its exported projection is bounded

The default exporter SHALL use Pydantic v2 TypeAdapter in validation mode with input aliases. The package SHALL expose Pydantic through an optional `types` extra; ordinary import and custom-exporter use SHALL NOT import or require it. Missing optional support SHALL identify the installation extra without masking unrelated import failures.

The supported path SHALL cover JSON-representable records, scalar fields, sequences, string-keyed mappings, nullable/alternative values, literals/enums, declarative constraints and static serializable defaults, plus representable standard-library date/time/datetime annotations and static temporal defaults. It SHALL return completed values through existing APIs, including dedicated DAML temporal values for temporal literals/defaults, not construct Python model instances or adopt Pydantic coercions. DAML's existing required-container completion and mathematical JSON integer semantics remain unchanged.

Temporal projection SHALL emit the shared DAML scalar-kind/offset constraints and explicit temporal-default metadata rather than silently reinterpret RFC-only format annotations as the ISO temporal contract. Plain datetime permits an absent or explicit offset; Pydantic's framework-owned AwareDatetime/NaiveDatetime annotations SHALL require/forbid it. Arbitrary custom type/schema hooks SHALL remain unsupported. Temporal bounds, relative-now rules, fixed-offset constraints or other core behavior not represented by the delivered constraints SHALL fail with binding/context rather than disappear. Default factories SHALL not be invoked. Temporal default metadata SHALL follow actual static temporal values; a date-looking string default SHALL not be silently promoted.

#### Scenario: Base-only installation uses a custom exporter
- **WHEN** Pydantic is unavailable and a caller supplies an exporter
- **THEN** base import and export/composition/resolution succeed without importing Pydantic

#### Scenario: Unrepresentable Python behavior is rejected
- **WHEN** a model uses a custom validator, default factory, custom serialization/schema override, normalization transform, post-init/custom-init behavior or unsupported core-schema form
- **THEN** the default export fails explicitly with a public exception identifying the affected binding/context rather than omitting the behavior or invoking the factory

#### Scenario: Temporal format is not silently discarded
- **WHEN** a caller-supplied exported schema uses an unasserted format at a schema position, including below definitions or references
- **THEN** export fails explicitly rather than dropping the assertion or treating it as a temporal constructor
- **AND** an ordinary property named format or literal default data containing that key is not mistaken for a format keyword

#### Scenario: Temporal model and dataclass defaults retain intrinsic kinds
- **WHEN** a model or dataclass declares standard date/time/datetime fields with actual static temporal defaults
- **THEN** deriving, composing and resolving omitted fields yields the corresponding DAML temporal objects and exact ISO text
- **AND** supplied intrinsic temporal values override defaults while incompatible kinds and ordinary strings fail

#### Scenario: Offset requirements survive derivation
- **WHEN** fields use the framework-owned AwareDatetime or NaiveDatetime annotation
- **THEN** required or forbidden offset policy is enforced on the admitted DAML datetime
- **AND** plain datetime supports both without assuming a machine timezone

#### Scenario: Unsupported temporal comparison behavior is not weakened
- **WHEN** a temporal field carries an unrepresented bound, relative-now rule or fixed-offset requirement
- **THEN** derivation fails explicitly with the affected binding/context before returning a weakened module
