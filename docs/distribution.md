# Install DAML command-line tools

DAML provides `daml-lsp` and `daml-fmt` as native executables. Release archives are public GitHub release assets and do not require an account or authentication. The current release page lists immutable versioned archives and SHA-256 sidecars for the supported targets.

## Supported release targets

| Operating system | Architecture | Rust target | Archive |
|---|---|---|---|
| Linux (Ubuntu 22.04 glibc baseline) | x86-64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| macOS | Apple Silicon (ARM64) | `aarch64-apple-darwin` | `.tar.gz` |
| Windows | x86-64 | `x86_64-pc-windows-msvc` | `.zip` |

This is the tested release matrix, not a claim about other systems. Linux releases use a glibc baseline and are not static-musl builds. The archives do not include signed installers or platform signing/notarization.

## Download and install a release

Open [DAML releases](https://github.com/dprb-work/daml/releases) and choose the version and archive for your operating system. Download both the archive and its `.sha256` sidecar. Verify the archive before extracting it.

On Linux:

```sh
archive=daml-0.1.0-x86_64-unknown-linux-gnu.tar.gz
base=https://github.com/dprb-work/daml/releases/download/v0.1.0
curl -fL "$base/$archive" -o "$archive"
curl -fL "$base/$archive.sha256" -o "$archive.sha256"
sha256sum -c "$archive.sha256"
mkdir -p "$HOME/.local/opt/daml"
tar -xzf "$archive" -C "$HOME/.local/opt/daml"
```

Add the extracted version's `bin` directory to your `PATH`. For example:

```sh
export PATH="$HOME/.local/opt/daml/daml-0.1.0-x86_64-unknown-linux-gnu/bin:$PATH"
daml-lsp --help
daml-fmt --help
```

Use `shasum -a 256 -c <archive>.sha256` on macOS. In PowerShell, download the
ZIP and sidecar from the matching public release URL. Compare
`Get-FileHash -Algorithm SHA256 <archive>` with the sidecar's digest, then use
`Expand-Archive` to extract the ZIP. Add the extracted `bin` directory to the
Windows `PATH`.

Keep the archive's `BUILD.json`, `LICENSE`, `SOURCES.md`, and third-party notices with the installation. A checksum detects accidental corruption; it does not by itself prove who published an asset. Use the GitHub release page as the source of the versioned assets.

## Select a version

Each release archive has a version and target in its filename and extracts into
a versioned directory. Keep multiple version directories when you need to switch
versions; point `PATH` or your editor's server setting at the selected
directory.

The LSP server starts a stdio session when launched without arguments. `--help` prints command usage and is not an LSP session. See the [language server guide](lsp.md) and [formatter reference](formatting.md) for their contracts.

## Build from source

If no release asset matches your system, build from a source checkout with the pinned Rust toolchain:

```sh
cargo build --locked --release --features lsp --bin daml-lsp --bin daml-fmt
```

The executables are under `target/release/` (with `.exe` suffixes on Windows). This source-build route does not produce the versioned release archive. For archive packaging and verification procedures, see the [distribution development guide](development/distribution.md).

The optional [Python API](python.md) is built separately from source; no Python wheel or PyPI release is provided. The [browser SDK](../bindings/browser/README.md) is distributed as a GitHub release asset, not through npm.