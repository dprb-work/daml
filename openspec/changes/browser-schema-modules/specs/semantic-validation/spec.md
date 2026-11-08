# Semantic constraint validation

## Purpose

Check supplied data against composed contracts without changing that data through defaults or container construction.

## ADDED Requirements

### Requirement: Supplied values validate without completion

The core, Python binding and browser SDK SHALL expose semantic constraint validation using the existing compiled schema registry. Validation SHALL NOT apply defaults, construct missing containers, invoke stages or mutate caller input.

#### Scenario: A required container is absent

- **WHEN** supplied data omits a required object or array that resolution could construct
- **THEN** validation rejects the supplied shape while ordinary resolution retains its completion behavior.

#### Scenario: Supplied completed data satisfies the contract

- **WHEN** the caller supplies all required fields with valid values
- **THEN** validation succeeds without changing the caller's semantic tree.

### Requirement: Validation enforces assertions and budgets

Validation SHALL check the explicit expected type and every supplied tag/binding assertion, including nested assertions. Existing numeric, UTF-8, key-size, node, depth and evaluation-work limits SHALL apply.

#### Scenario: A nested assertion fails under an unconstrained parent

- **WHEN** a nested supplied tag violates its own registered contract even though the parent accepts that value shape
- **THEN** validation fails at the nested instance pointer.

#### Scenario: An asserted or expected type is unknown

- **WHEN** the caller names an unregistered tag, binding or expected type
- **THEN** validation fails through the existing structured resolution-error boundary.

### Requirement: Constraint validation does not promise source representability

Validation SHALL retain literal JSON mapping keys without interpreting them as schema syntax. Its success SHALL NOT relax DAML source-key admission or serialization rules.

#### Scenario: A literal keyword key is data

- **WHEN** a supplied mapping contains a literal `$ref` field allowed by its contract
- **THEN** validation checks that field as ordinary data without rewriting or resolving its value.

#### Scenario: A valid JSON key is not a DAML source key

- **WHEN** constraint validation accepts such a mapping
- **THEN** ordinary semantic resolution and writing continue to enforce their existing source-key restrictions.
