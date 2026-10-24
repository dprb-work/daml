# Definitively Asserted DAML for VS Code and Cursor

This private extension provides `.daml` language detection, TextMate syntax highlighting, syntax-only diagnostics, and whole-document formatting through the released `daml-lsp` executable. It does not validate schemas or bundle semantics. The language id `definitively-asserted-daml` intentionally avoids the Digital Asset DAML extension's language id.

## Install the server

Download the `daml-0.1.0` asset for your operating system and architecture from the private [DAML v0.1.0 GitHub release](https://github.com/dprb-work/daml/releases/tag/v0.1.0). Install `daml-lsp` in a directory on `PATH`, or retain its absolute path. The editor extension never downloads, builds, or installs the server. Installation of the server and extension is separate.

## Build and install the VSIX

From this directory, with Node.js and npm available:

```sh
npm ci
npm run package
code --install-extension definitively-asserted-daml-0.1.0.vsix
```

To install in Cursor, use **Extensions: Install from VSIX...** and select that same file. VS Code-compatible Cursor releases can reuse the VSIX; no marketplace publication or global settings changes are needed. The declared VS Code engine baseline is `^1.85.0`.

## Executable setting and workspace trust

The extension uses `daml-lsp` from `PATH` by default. If it is elsewhere, set `definitivelyAssertedDaml.serverPath` to its absolute path in **user settings**. This is a machine-scoped setting, so workspace settings cannot substitute an executable. The executable is launched directly with no shell and no arguments.

The server does not start while the workspace is untrusted. Trusting the workspace starts the client; revoking trust is unavailable as a workspace transition in VS Code. Changing the executable setting in trusted state restarts the client. A missing executable produces an actionable error naming the configured command and the private release installation options.

## Package maintenance

`npm run package` bundles the extension and produces `definitively-asserted-daml-0.1.0.vsix`. It stages the canonical repository-root MIT `LICENSE` and generates `THIRD-PARTY-NOTICES.txt` from the esbuild metafile and license/notice files for packages actually included in the runtime bundle; both generated files are ignored and must not be maintained by hand. Dependencies are pinned in `package.json` and `package-lock.json`. Use `npm run typecheck` for strict TypeScript checking; it uses the VS Code 1.85 API types matching the declared engine baseline. Do not publish this package to a marketplace.

The build stages the pinned language client's upstream `terminateProcess.sh`
beside the bundled JavaScript, with executable permission. Keep that runtime
asset when changing bundlers: the standard client uses it to terminate lingering
server process trees on Linux/macOS after shutdown.

The grammar is editor presentation only. It follows constructs described in the repository's [`native authoring contract`](https://github.com/dprb-work/daml/blob/v0.1.0/docs/native-authoring.md) and [`syntax contract`](https://github.com/dprb-work/daml/blob/v0.1.0/docs/syntax.md); it is not a parser or semantic validator. Diagnostics and formatting are solely supplied by the existing language server.

### Temporary build-tool security exception

The manifest scopes an MSAL Node `7.1.0` override to VSCE `4.0.0` through Azure
Identity `4.14.0`, fixing [GHSA-xcx6-jx8p-x3h5](https://github.com/advisories/GHSA-xcx6-jx8p-x3h5).
This is an explicitly accepted compatibility exception: Identity still declares
MSAL `^6.0.0`; no released parent currently admits the patched version.

The [MSAL v7 migration guide](https://github.com/AzureAD/microsoft-authentication-library-for-js/blob/msal-node-v7.1.0/lib/msal-node/docs/v7-migration.md)
identifies removal of interactive `responseMode`; the inspected Identity calls
do not supply it. Static API review and packaging/editor checks do not establish
every Azure credential or authenticated publishing flow. The high-severity npm
audit remains enforced without exclusions.

Keep the parent-version selectors exact. Remove this exception when a supported
parent release admits patched MSAL rather than widening it to unreviewed parents.
