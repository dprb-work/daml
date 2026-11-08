## Purpose

Let users consume DAML's Python configuration APIs from verified native wheels without a source checkout or Rust build tools, while retaining honest compatibility, license and source identity information.

## ADDED Requirements

### Requirement: Distribution identity and stable import

The Python distribution SHALL be named `daml-config` and SHALL retain `daml` as its import package. Its version SHALL remain owned by the Python Cargo manifest. The base install SHALL NOT require Pydantic; the existing `types` extra SHALL provide the supported Pydantic/dataclass schema-export path.

#### Scenario: Independent base install

- **WHEN** a compatible wheel is installed without dependencies into a new environment
- **THEN** `import daml` and ordinary parsing work without Pydantic, project source paths or Rust tools
- **AND** installed distribution metadata identifies `daml-config` at the candidate's declared version

### Requirement: Honest native compatibility

Candidate wheels SHALL target the existing Linux x64, macOS ARM64 and Windows x64 platform matrix using the CPython 3.12 stable ABI. Linux wheels SHALL have an audited PyPI-compatible manylinux tag with a glibc floor no newer than 2.35. Unsupported hosts or audit failures SHALL fail rather than produce misleading platform names. macOS deployment requirements SHALL match the actual build. No PyPy or free-threaded compatibility is implied.

#### Scenario: Later compatible interpreter

- **WHEN** the same platform wheel is installed on standard CPython 3.12 and 3.13
- **THEN** both environments load the wheel's native module and complete the consumer workflow
- **AND** neither installation compiles DAML from source

### Requirement: Legal payload and revision-bound evidence

Each wheel SHALL contain the canonical project MIT license, source inventory, dependency notices generated from the Python extension's admitted Cargo graph, and build metadata naming the full Git revision, dirty-source state, platform, compiler, backend and package version. Every candidate wheel SHALL have a matching SHA-256 sidecar. Release qualification SHALL use a clean committed source candidate.

#### Scenario: Install an identified artifact

- **WHEN** a wheel is verified for installation
- **THEN** its checksum and required package, typing, legal and build metadata are checked before execution
- **AND** the verifier reports the artifact identity and observed installed-module location

#### Scenario: Tampered or incomplete artifact

- **WHEN** the artifact's bytes disagree with its sidecar, its archive members are unsafe or duplicated, or its required identity/legal payload is missing or inconsistent
- **THEN** verification fails before installing it

### Requirement: Isolated type-derived configuration workflow

The installed-wheel verifier SHALL run outside the checkout in a new temporary environment, prevent editable-source/PYTHONPATH leakage, require binary-only dependency installation, and clean up the temporary installation on success and failure. Its retained result SHALL describe the actual observed consumer outcomes.

#### Scenario: Defaults and supplied values

- **WHEN** the installed `types` extra derives a schema from either the documented Pydantic model or equivalent standard dataclass
- **THEN** omitted retries resolve to `3`, supplied retries `5` remain `5`, and supplied string retries fail at `/retries`
- **AND** successful resolution returns ordinary dictionary values rather than model instances

### Requirement: Preparation is not publication

Build and CI commands SHALL prepare and verify artifacts without uploading packages, creating release tags, changing visibility or promoting release assets. Documentation SHALL distinguish unpublished candidate-wheel installation from source builds and actual published release availability.

#### Scenario: Candidate build

- **WHEN** the packaging procedure succeeds locally or in CI
- **THEN** it produces a wheel and checksum with verification evidence
- **AND** it does not imply that a PyPI project or public release asset has been published
