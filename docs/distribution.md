# Build and install binary distributions

DAML's Binaries workflow builds native archives containing `daml-lsp` and
`daml-fmt`. Users run the extracted executables; Rust, Cargo, Python, and Node.js
are build/verification tools, not DAML runtime requirements. Native operating
system libraries still apply.

Artifacts remain within the private repository. This workflow does not publish
GitHub releases, push tags, publish a crate, or install an editor integration.
DAML's native distributions carry the MIT license and generated dependency notices.

## Build matrix and downloads

The automatic `Check` workflow selects native archive verification for pull
requests and pushes to `main` when source, package metadata, toolchain, or archive
inputs affect the distribution contract. Draft pull requests skip archive builds.
Manual `Check` dispatch selects the full CI plan; the standalone `Binaries`
workflow remains a manual archive-verification entrypoint. Each selected native
job builds and exercises the archive on its runner:

| Runner | Rust target | Archive |
|---|---|---|
| Ubuntu 22.04 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| macOS 15, Apple Silicon | `aarch64-apple-darwin` | `.tar.gz` |
| Windows 2022, x64 | `x86_64-pc-windows-msvc` | `.zip` |

This is the initial supported build matrix, not a claim about untested targets.
Linux CI uses Ubuntu 22.04's glibc baseline; it is not a static-musl build. A local
build uses the local OS/toolchain baseline. macOS signing/notarization and Windows
signing/installers are not provided by this slice.

Open a successful **Binaries** Actions run and download its `daml-<target>`
artifact. With authenticated GitHub CLI access to this private repository:

```sh
gh run download RUN_ID --repo dprb-work/daml \
  --name daml-x86_64-unknown-linux-gnu --dir download
```

The GitHub artifact is an outer transport ZIP. Its contents are the DAML archive
and matching `.sha256` sidecar. Artifacts expire after 14 days; they are CI
snapshots, not permanent release URLs. Inspect the run revision and `BUILD.json`
before using a snapshot. A checksum detects corruption, not publisher identity;
repository access and the selected workflow run establish the source.

## Private releases

Versioned archives and checksum sidecars are published as assets of private
[GitHub releases](https://github.com/dprb-work/daml/releases). They retain the
repository's access controls and do not expire with Actions artifacts:

```sh
gh release download --repo dprb-work/daml \
  --pattern 'daml-*-x86_64-unknown-linux-gnu.tar.gz*' --dir download
```

Select the corresponding target for macOS or Windows. Release assets come from
a successful Binaries run of the release commit. `BUILD.json` must identify that
commit and a clean source tree. The workflow builds and verifies archives;
promoting those artifacts to a release is a separate authorized action.
Crate/PyPI publication and Python wheel/sdist assets are outside this route.

## Build the same archive locally

Use the pinned Rust toolchain, Python 3.11 or later, and the pinned notice tool:

```sh
cargo install cargo-about --version 0.9.2 --locked --features cli
python3 scripts/package-binaries.py --out dist
```

The script can be invoked from another cwd with its absolute path. It uses the
repository beside the script, native `rustc` host target, locked dependencies,
release profile, and the `lsp` feature. `CARGO_TARGET_DIR` is honored. Cross-target
labeling is not inferred from a caller-provided filename or build-target setting.

Each archive has one `daml-<version>-<target>/` directory containing:

```text
bin/daml-lsp       # .exe on Windows
bin/daml-fmt       # .exe on Windows
BUILD.json
LICENSE            # canonical root MIT license
SOURCES.md
THIRD-PARTY-NOTICES.txt
```

`BUILD.json` records package version, full Git commit, dirty-source flag, target,
Rust compiler version, and enabled build feature. Dirty local builds are marked
as such; clean CI snapshots identify a committed source tree. Third-party notices
are generated with `cargo-about` using the accepted licenses in `deny.toml`, not a
second allowlist. Build dependencies are included conservatively; dev/fuzz
packages are not part of this native distribution.

Archive members, permissions, timestamps, and gzip headers are normalized. Given
identical binary/notice/metadata payloads, packaging is deterministic. This does
not promise bit-identical compilation across different machines or environments.
No tool is silently installed by the packaging script.

## Install without a Rust checkout

On Linux, verify and extract the downloaded archive into an application directory:

```sh
cd download
sha256sum -c daml-0.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf daml-0.1.0-x86_64-unknown-linux-gnu.tar.gz
export PATH="$PWD/daml-0.1.0-x86_64-unknown-linux-gnu/bin:$PATH"
daml-lsp --help
daml-fmt --help
```

Use `shasum -a 256` to compare the macOS archive with its sidecar; use PowerShell
`Get-FileHash -Algorithm SHA256` and `Expand-Archive` for Windows. The version and
target names must match the downloaded archive. Keep the metadata and notices
with the installed executables. `daml-lsp` starts stdio service with no arguments;
`--help` is not a server mode.

The [LSP guide](lsp.md) describes the protocol capabilities. Separate
[editor/host packages](../integrations/README.md) provide VS Code/Cursor and
OMP/DSH integration; they are not part of the native archives. An OMP project/root
marker locates its server process, not a DAML schema root.

## Verify the installed archive

The project-local [distribution verification skill](../skills/verify-daml-distribution/SKILL.md)
owns repeatable installation and isolated OMP driving. CI reuses existing
formatter/LSP process contracts against executables extracted to a new temporary
installation, through `DAML_TEST_BIN_DIR`; it does not merely test the build-tree
binaries.

```sh
python3 skills/verify-daml-distribution/scripts/archive.py \
  dist/daml-0.1.0-x86_64-unknown-linux-gnu.tar.gz
```

To drive the installed OMP executable without touching host configuration:

```sh
python3 skills/verify-daml-distribution/scripts/archive.py \
  download/daml-0.1.0-x86_64-unknown-linux-gnu.tar.gz \
  --omp /absolute/path/to/omp --evidence .scratch/distribution/omp
```

OMP verification uses its real RPC process and bundled built-in LSP tool, not an
import from the preserved OMP source checkout. A verification-only temporary
extension drives capabilities, Unicode syntax diagnostics, replacement/clearing,
formatting, edit application, idempotence, and the installed formatter. It uses
an isolated home/config/cache/data tree, disables shared LSP state, omits host
credentials, and sends no model prompt. The selected catalog model and non-secret
placeholder key only satisfy CLI startup. The probe is not an OMP integration
package. Retained evidence states the exact scope and OMP version; no interactive
TUI or GUI-editor claim is implied.

The [Python extension](python.md) is a separate package and build graph; it does
not change these native archives or add a Python runtime requirement to them.
The [plan](../PLAN.md#next-sequence) owns remaining language and conversion work.
The [integration guide](../integrations/README.md) owns client packaging and verification.
