## Why

DAML's Python-first onboarding currently requires a Rust toolchain and source checkout. Prepare installable binary wheels so users can try type-derived schemas, authored configuration, default completion and validation with Python alone.

## What Changes

- Rename the Python distribution to `daml-config`, the name selected by the user after confirming that PyPI's `daml` project belongs to another library. Keep `import daml` and the native module/API unchanged. Update all in-repository source-install selectors.
- Build locked, release-profile `cp312-abi3` wheels for Linux x64, macOS ARM64 and Windows x64 using the existing native runner matrix. Admit only audited, honestly tagged platform artifacts.
- Include the canonical MIT license, source inventory, generated Cargo dependency notices and revision-bound build provenance in every wheel, with a SHA-256 sidecar.
- Extend the existing distribution verification owner with isolated base-package and `types`-extra installs outside the checkout, exercised on CPython 3.12 and 3.13.
- Retain candidate wheels as CI artifacts and document local/candidate installation. No PyPI upload, release asset promotion, credentials, tags or merge in this preparation task.

## Capabilities

### New Capabilities

- `python-wheel-distribution`: Native Python wheels with legal/provenance payloads and independently owned installed-consumer verification.

### Modified Capabilities

None. Schema projection, parsing, defaults, validation and model-instance behavior do not change.

## Impact

Python metadata/lockfile and source-install commands, a repository-owned wheel producer, the existing native distribution matrix, the project-local distribution verifier, and installation documentation change. Rust/native CLI/browser package names and versions do not change. This is the first prepared `daml-config` distribution at the existing Python Cargo-owned version `0.1.0`; it does not replace another project's PyPI package.

The import package remains `daml`; this package and the unrelated PyPI distribution named `daml` must not be installed together. Standard CPython 3.12+ is the stable-ABI target; PyPy and free-threaded builds are not claimed. No sdist release or new architecture is promised. Recursive-type PR #26 remains separate.

## Scope decision

The user authorized the bounded packaging pass with “aight go” and selected `daml-config` through the package-name decision. Artifact publication remains a separate approval. Exact candidate result acceptance and merge remain pending verification and review.
