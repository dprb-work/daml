# Installed Python wheel

A user installs the unpublished `daml-config` distribution and keeps `import daml`.
The native extension uses the existing PyO3 `abi3-py312` contract: standard
CPython 3.12+, not PyPy or free-threaded Python. Supported native targets are
Linux x64/glibc (audited manylinux, glibc floor no higher than 2.35), macOS ARM64
(explicit 11.0 deployment target), and Windows x64/MSVC.

## Reach and drive

Produce a candidate with `python scripts/package-python.py --out dist/python`.
Do not publish it. From the repository root run:

```sh
python skills/verify-daml-distribution/scripts/wheel.py /absolute/path/to/CANDIDATE.whl
```

A matching `CANDIDATE.whl.sha256`, current producing source revision, Git, uv,
and standard CPython 3.12+ are prerequisites. Optional `--python INTERPRETER`
selects an already installed interpreter; automatic downloads are disabled.
The wheel and sidecar should be from the intended producer/workflow, not an
unknown publisher. Network access is needed only for binary types dependencies.

## Doctor before installation

The helper rejects a candidate before `uv pip install` if it fails:

- Exact SHA-256 sidecar content and filename; safe, regular, unencrypted,
  nonduplicate ZIP members (including case collisions and file/directory clashes).
  Original entry names must be canonical and unchanged by ZIP filename
  normalization; embedded NULs and backslash separators are rejected on every OS.
- Complete SHA-256/size RECORD coverage, matching distribution/version metadata,
  stable-ABI filename/WHEEL tags, minimum Python, and optional `types` metadata.
- Current source bytes for the import package, native stub, `py.typed`, and
  type-schema adapter; no extra import roots, `.pth` hooks, or runtime payload.
- Canonical root MIT `LICENSE` and `SOURCES.md` bytes at `daml/_distribution/`,
  generated-notice format, license texts, and locked PyO3/serde_json notices.
- `BUILD.json` version, distribution, commit, dirty boolean, release profile,
  pinned maturin, Rust identity, native target, deployment target, requested
  audit/compatibility mode, and canonical Python notices-generation provenance.
- Matching ELF/Mach-O/PE architecture, manylinux GLIBC symbol-reference ceiling,
  or Mach-O minimum OS. The selected interpreter's platform/architecture and
  Python implementation must match before installation.

These checks detect inconsistencies; the verifier does not regenerate Cargo
notices, prove the build history, or replace producer audit evidence. A matching
sidecar is integrity evidence, not authentication. This is a narrow DAML package
procedure, not a hostile-code sandbox.

## Observable consumer contracts

One fresh temporary uv environment lives outside the checkout. Both installs
name a copied snapshot of the exact checked wheel, with `--only-binary :all:`
forbidding source builds; Cargo/Rust are absent from the installer PATH. Base
installation also uses `--no-index --no-deps`. Consumers run with `python -I`,
no inherited PYTHON/uv configuration, and an external working directory.

1. Base: Pydantic cannot be discovered; `import daml` and `daml.parse(...).to_python()`
   return the expected plain dictionary without the extra.
2. Types: install the exact wheel's `[types]` extra, then use actual Pydantic and
   standard dataclass workers through `schema_from_types`, `Schemas.compose`,
   `parse`, `resolve`, and `to_python`.
3. Both workers complete omitted retries to **3**, preserve an explicit **5**, and
   reject a string at **`/retries`**. Successful results are ordinary JSON-shaped
   dictionaries, never constructed model instances.
4. Package, native extension, adapter, and distribution metadata resolve inside
   the new venv; no checkout path is present in the consumer import search path,
   and no editable install is admitted.

Source anchors: `bindings/python/pyproject.toml`, `bindings/python/Cargo.toml`,
`bindings/python/python/daml/`, `docs/python-quickstart.md`, and the canonical
root legal files. Existing root runners/assertions/fixtures remain unchanged.

## Evidence, isolation, cleanup

Each invocation owns its entire scratch tree and environment; parallel invocations
share neither. Stdout contains one JSON receipt with candidate SHA-256, build,
interpreter, module origins, base result, each worker's defaults/override/error
path, and final cleanup state. Subprocess output goes to stderr. Preserve stdout
outside scratch (for example as a CI log/artifact). Failures return nonzero and a
failed receipt; context-managed teardown removes scratch on failure as well as
success. Receipts retain paths as evidence even after those paths are removed.
