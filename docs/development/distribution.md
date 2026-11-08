# Build and install binary distributions

DAML's Binaries workflow builds native archives containing `daml-lsp` and
`daml-fmt`, plus separate Python wheel candidates. Users of the native
executables need no Rust, Cargo, Python or Node.js; native OS libraries still
apply. Python wheel users need a compatible Python interpreter, not Rust.

Native archives are available through GitHub Actions and public GitHub releases.
Python wheels are candidate Actions artifacts only. The workflow uploads no
crate, PyPI package, sdist or release asset. See the
[user installation instructions](../distribution.md) for native releases and
[candidate-wheel installation](../python.md#install-a-candidate-wheel) for Python.

## Build matrix and downloads

The automatic `Check` workflow selects native archive verification for pull requests and pushes to `main` when source, package metadata, toolchain, or archive inputs affect the distribution contract. Draft pull requests skip archive builds. Manual `Check` dispatch selects the full CI plan; the standalone `Binaries` workflow remains a manual archive-verification entrypoint. Each selected native job builds and exercises the archive on its runner:

| Runner | Rust target | Archive |
|---|---|---|
| Ubuntu 22.04 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| macOS 15, Apple Silicon | `aarch64-apple-darwin` | `.tar.gz` |
| Windows 2022, x64 | `x86_64-pc-windows-msvc` | `.zip` |

This is the recorded initial build matrix, not a claim about untested targets. Linux CI uses Ubuntu 22.04's glibc baseline; it is not a static-musl build. A local build uses the local OS/toolchain baseline. Signing, notarization, and Windows installers are not provided by this workflow.

A successful **Binaries** Actions run contains a `daml-<target>` artifact. The repository's access controls and selected workflow run establish the source; use the public release page for versioned downloads.

```sh
gh run download RUN_ID --repo dprb-work/daml \
  --name daml-x86_64-unknown-linux-gnu --dir download
```

The GitHub artifact is an outer transport ZIP containing the DAML archive and matching `.sha256` sidecar. Artifacts expire after 14 days; they are CI snapshots, not permanent release URLs. Inspect the run revision and `BUILD.json` before using a snapshot. A checksum detects corruption, not publisher identity; repository access and the selected workflow run establish the source.

Versioned archives and checksum sidecars are published as assets of public GitHub releases. They do not expire with Actions artifacts:

```sh
gh release download --repo dprb-work/daml \
  --pattern 'daml-*-x86_64-unknown-linux-gnu.tar.gz*' --dir download
```

Select the corresponding target for macOS or Windows. Release assets come from a successful Binaries run of the release commit. `BUILD.json` must identify that commit and a clean source tree. The workflow builds and verifies archives; promoting those artifacts to a release is a separate authorized action.

## Build the same archive locally

Use the pinned Rust toolchain, Python 3.11 or later, and the pinned notice tool:

```sh
cargo install cargo-about --version 0.9.2 --locked --features cli
python3 scripts/package-binaries.py --out dist
```

Run these commands from the repository root. The script can also be invoked from another working directory with its absolute path. It uses the repository beside the script, the native `rustc` host target, locked dependencies, the release profile, and the `lsp` feature. It honors `CARGO_TARGET_DIR`. It does not infer a cross target from a caller-provided filename or build-target setting.

Each archive has one `daml-<version>-<target>/` directory containing:

```text
bin/daml-lsp       # .exe on Windows
bin/daml-fmt       # .exe on Windows
BUILD.json
LICENSE            # canonical root MIT license
SOURCES.md
THIRD-PARTY-NOTICES.txt
```

`BUILD.json` records package version, full Git commit, dirty-source flag, target, Rust compiler version, and enabled build feature. Dirty local builds are marked as such; clean CI snapshots identify a committed source tree. `cargo-about` generates third-party notices using the accepted licenses in `deny.toml`, not a second allowlist. Build dependencies are included conservatively; dev and fuzz packages are not part of this native distribution.

Archive members, permissions, timestamps, and gzip headers are normalized. Given identical binary, notice, and metadata payloads, packaging is deterministic. This does not promise bit-identical compilation across different machines or environments. The packaging script does not silently install tools.

## Verify the installed archive

The project-local [archive verification guide](../../skills/verify-daml-distribution/features/binary-archive.md) exercises the installed executables. CI runs the existing formatter/LSP process contracts against executables extracted to a temporary installation, through `DAML_TEST_BIN_DIR`; it does not merely test build-tree binaries.

From the repository root, run:

```sh
python3 skills/verify-daml-distribution/scripts/archive.py \
  dist/daml-0.1.0-x86_64-unknown-linux-gnu.tar.gz
```

The [LSP guide](../lsp.md) describes protocol capabilities. The VS Code extension is separate from native archives. The [Python extension](../python.md) has a separate package and build graph; it does not change these archives or add a Python runtime requirement.

## Python wheel candidates

The same three native jobs prepare the `daml-config` distribution. Its import
package remains `daml`; the unrelated PyPI project named `daml` is not this
distribution. The Python Cargo manifest owns version `0.1.0` independently of
the older native release archives.

| Platform | Wheel contract |
|---|---|
| Linux x64 | Audited `manylinux_2_35_x86_64`; glibc 2.35 minimum, not musl |
| macOS ARM64 | `macosx_11_0_arm64`; explicit macOS 11.0 deployment target |
| Windows x64 | Native MSVC `win_amd64` |

All three use the existing PyO3 `abi3-py312` feature. CI installs the same wheel
on standard CPython 3.12 and 3.13; PyPy and free-threaded compatibility are not
claimed. Linux audit failure does not fall back to an unverified `linux` tag.

### Prepare a wheel locally

Use a supported native host, the pinned Rust toolchain, uv and cargo-about
0.9.2. Install the notice generator if it is not already present:

```sh
cargo install cargo-about --version 0.9.2 --locked --features cli
```

From the repository root, invoke the producer with its pinned backend:

```sh
uv run --no-project --python 3.12 --with maturin==1.15.0 \
  python scripts/package-python.py --out dist/python
```

The explicit uv command prepares a build environment; the producer itself
installs no tools. Use an output directory with no existing wheel or wheel
sidecar candidates. It refuses pre-existing package staging rather than
overwriting it.

`scripts/package-python.py` is the canonical wheel producer. It builds with
locked dependencies in release mode, includes the four-file payload below and
writes the wheel plus its `.whl.sha256` sidecar:

```text
daml/_distribution/BUILD.json
daml/_distribution/LICENSE
daml/_distribution/SOURCES.md
daml/_distribution/THIRD-PARTY-NOTICES.txt
```

The root MIT license and source inventory are copied only into temporary
generated staging. `scripts/cargo-notices.py` derives the Python graph's notices
from the shared license policy. Maturin owns wheel contents and `RECORD`; there
is no manual wheel-repacking step. Staging is removed on success and failure;
ordinary editable/source installs do not require it.

`BUILD.json` records the Python distribution/version, Git commit and dirty
state, native target, compiler, backend, profile, compatibility audit and notice
generator. Local dirty builds are qualification artifacts; release promotion
requires a clean committed candidate. Fixed metadata and packaging inputs do
not promise bit-identical compilation on different machines.

### Verify the installed wheel

Run the [project-local wheel verification procedure](../../skills/verify-daml-distribution/features/python-wheel.md)
against the exact wheel and sidecar. For example, on Linux:

```sh
uv run --no-project --python 3.12 python \
  skills/verify-daml-distribution/scripts/wheel.py \
  dist/python/daml_config-0.1.0-cp312-abi3-manylinux_2_35_x86_64.whl
uv run --no-project --python 3.13 python \
  skills/verify-daml-distribution/scripts/wheel.py \
  dist/python/daml_config-0.1.0-cp312-abi3-manylinux_2_35_x86_64.whl
```

The helper checks artifact identity/legal payloads and installs into a new
temporary environment outside the checkout. It exercises base parsing without
Pydantic, then installs the binary-only `types` extra and runs the default,
override and invalid-value cases through both Pydantic and dataclasses.
Isolated execution checks that imports come from the installation, not source
paths. The helper removes its temporary environments and prints evidence.

### Retain candidates without publication

The workflow stores wheel/checksum pairs in a `daml-config-<Rust target>`
Actions artifact for 14 days. A successful build does not create a PyPI project,
release tag or permanent download. Promotion requires separate authorization
for the exact verified revision and files. Python sdists are not part of this
qualified distribution route.
