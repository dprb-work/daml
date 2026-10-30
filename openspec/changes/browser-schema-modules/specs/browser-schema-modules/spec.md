# Browser schema modules

## Purpose

Let browser consumers reuse generated JSON Schema contracts through DAML's existing registry instead of generating native definition text.

## ADDED Requirements

### Requirement: Browser consumers compose explicit JSON modules

The browser SDK SHALL expose `Schemas.fromJsonModules(modules: readonly string[])` and admit each module through the existing strict JSON Schema interchange boundary.

#### Scenario: Cross-module resources preserve native results

- **WHEN** native and browser consumers compose the same module strings with explicitly registered cross-module resources
- **THEN** their resolution, exact numeric defaults, canonical output and default provenance agree.

#### Scenario: Missing or duplicate contracts fail composition

- **WHEN** a module references an unregistered resource or composition repeats a type/resource identity
- **THEN** composition fails with a structured schema error rather than choosing a winner or fetching a resource.

### Requirement: Browser admission remains bounded and failure preserving

The browser SDK SHALL bound module count to 10,000 and cumulative UTF-8 module text to 8 MiB before retaining unbounded input. Existing core schema, depth and compile-work limits SHALL remain enforced.

#### Scenario: Aggregate text exceeds the bound

- **WHEN** individually valid module texts exceed the cumulative byte limit
- **THEN** admission fails without returning a partial schema context.

#### Scenario: Foreign array access throws

- **WHEN** a module-array length or element accessor throws
- **THEN** the original thrown JavaScript value propagates unchanged.

#### Scenario: Input cannot preserve its original text

- **WHEN** a module entry is not a primitive string or contains an unpaired UTF-16 surrogate
- **THEN** admission fails rather than coercing or replacing the input.

### Requirement: Compiled contexts retain their inputs independently

A returned schema context SHALL own its admitted contracts independently of the caller's array. Existing explicit handle cleanup and owned resolution snapshots SHALL continue to apply.

#### Scenario: Caller input or handles are released

- **WHEN** the caller changes its module array and later frees the source document and schema context
- **THEN** completed resolution snapshots remain valid and retain their original semantic value.
