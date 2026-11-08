# Bounded structural schema recursion

## Purpose

Preserve finite recursively nested consumer data without weakening its schema or bypassing resource limits. The scope owner accepted this as a required integration capability in the conversation: “yes, that is a required capability”.

## ADDED Requirements

### Requirement: Structural recursion preserves finite data contracts

DAML SHALL compose explicit schema modules containing recursive references whose evaluation cycles make strict progress into instance children. Native, Python and browser consumers SHALL use the same supported core semantics. Referenced resources, canonical bindings, local definition scope and existing constraints SHALL remain intact. Application names and provider registries SHALL NOT receive special treatment.

#### Scenario: A recursively nested object or collection is finite

- **WHEN** an explicitly registered contract recurs through an object member or collection element and supplied finite data satisfies every reachable constraint
- **THEN** native and browser admission accept the value without truncating the contract or fetching resources.

#### Scenario: A nested recursive value violates a constraint

- **WHEN** a supplied descendant violates a recursive field constraint
- **THEN** validation rejects it through the existing structured error boundary and preserves the applicable instance pointer.

### Requirement: Non-progressing evaluation loops remain invalid

Composition SHALL reject evaluation cycles that can recur on the same instance without consuming an instance child. Merely containing a definition is not applying that definition to an instance. Dynamic reference features remain outside this change.

#### Scenario: References loop through same-instance applicators

- **WHEN** a reference cycle through references, alternatives, conjunction, negation or same-instance conditional/dependency application can revisit the same instance without descent
- **THEN** composition fails with the existing reference-cycle error rather than relying on stack exhaustion or treating the loop as success.

### Requirement: Recursive work and completion remain bounded

Recursive schema composition, validation and completion SHALL retain the existing physical node, byte, numeric, depth and work budgets. Validation SHALL establish a sound work bound before entering unbounded upstream evaluation or use a proven equivalent bounded engine mechanism. Repeated recursive alternatives SHALL NOT bypass accounting. Default and uniqueness reachability SHALL remain correct across recursive components.

#### Scenario: Defaults complete a finite recursive value

- **WHEN** supplied recursive data has finite leaves and supported applicable defaults
- **THEN** resolution applies the same defaults with their real schema provenance while constraint-only validation inserts nothing.

#### Scenario: Completion would construct forever

- **WHEN** required containers or recursive defaults demand unending construction
- **THEN** resolution fails within the existing resource bounds, without a panic, invented leaf, silent truncation or caller mutation.

#### Scenario: Recursive branch expansion exceeds the work budget

- **WHEN** a finite supplied value would require excessive recursive schema evaluation
- **THEN** admission fails with the existing limit error before the upstream engine performs that excessive work.
