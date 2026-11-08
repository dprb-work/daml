## Boundary and existing owners

Maturin 1.15.0 remains the sole wheel builder; the Python Cargo manifest owns its version and the existing PyO3 `abi3-py312` feature owns the ABI floor. The distribution changes to the user-selected `daml-config`, while Python/native import names stay unchanged. No alias distribution is added.

The repository wheel producer owns required tool checks, legal/provenance staging, locked release builds and sidecars. It reuses `scripts/cargo-notices.py` and `deny.toml`; it does not rewrite wheel ZIP/RECORD metadata or maintain a second license policy. Canonical root legal files stay canonical. Generated staging must not become another tracked copy or break ordinary source installs.

The existing Binaries reusable workflow owns the three native runners. Add wheel builds and installed-wheel verification there rather than introducing another independent platform matrix or publication controller. Existing draft/readiness, immutable candidate/base, permissions and aggregate policies remain intact. No paid plan or budget setting changes.

The project-local `verify-daml-distribution` skill owns artifact admission and installed-consumer verification. A separate check owner authors its expected outcomes and inputs; producer authors may read/run but not rewrite those checks. Shared-worktree ownership is procedural, not proven write isolation.

## Compatibility and qualification

Build once per platform and exercise that wheel under standard CPython 3.12 and 3.13. Linux must pass the wheel backend's compatibility audit at or below the existing Ubuntu 22.04 glibc 2.35 baseline; native `linux` tags or unchecked filename relabeling are not substitutes. macOS and Windows proof comes from their actual runners, not local cross-compilation claims.

Wheel verification installs the exact local artifact in a temporary environment outside the checkout, uses isolated Python execution, checks installed-module ownership and binary-only installs, and exercises both base parsing without Pydantic and the existing type-derived workflow with the extra. This tests the user path that source-tree tests cannot establish.

## Alternatives not selected

- Publishing under `daml`: occupied by an unrelated project; the user chose `daml-config`.
- Renaming `import daml`: unnecessary API migration; no concrete need in the accepted scope. Document that the two distributions cannot coexist.
- Per-minor wheels: the already-selected stable ABI avoids rebuilding identical bindings for each standard CPython minor.
- A new standalone release system or automatic uploads: no accepted need or authority. Retain qualified CI artifacts; publication is separately authorized.
- A broad sdist release effort: not needed to provide binary installation. Preserve the existing source-build path without claiming new source archives are release-qualified.

## Documentation and recovery

Update source-install package selectors and distinguish candidate wheels from published packages. Do not advertise `pip install daml-config` as a working registry route before publication. Native and browser downloads retain their existing contracts.

No service or persisted data migration occurs. A failed build/verification publishes nothing; scratch installations/staging are removed by their owners. Users qualify exact wheel/checksum pairs in fresh environments and can remove that environment without changing the repository. Release withdrawal or PyPI recovery is outside this no-publication task.
