# Definitively Asserted DAML for VS Code-compatible editors

The source-buildable VSIX provides `.daml` and `daml.bundle` detection, TextMate highlighting, diagnostics and formatting through `daml-lsp`. Matching current source builds add standard semantic tokens and explicitly selected semantic diagnostics, completion, hover and definition navigation. Highlighting needs no selected schema context; validation is syntax-only by default. Clients and user themes choose token colors. The language id `definitively-asserted-daml` is distinct from Digital Asset's DAML extension.

## Install the language server

Download the versioned `daml-lsp` archive for your operating system from the public [DAML releases](https://github.com/dprb-work/daml/releases), verify its SHA-256 sidecar, and add its `bin` directory to `PATH`. See the [command-line installation guide](https://github.com/dprb-work/daml/blob/main/docs/distribution.md) for platform-specific steps. The extension does not download, build, or install the server.

Older released servers may lack semantic providers. For current semantics, build the server from the same checkout as this extension:

```sh
cargo build --locked --features lsp --bin daml-lsp
```

Run this at the repository root, then set the machine's user setting below to the absolute `target/debug/daml-lsp` path (`.exe` on Windows). The extension does not upgrade an installed release automatically.

## Build and install the VSIX

From this directory, with Node.js and npm available:

```sh
npm ci --ignore-scripts
npm run package
code --install-extension definitively-asserted-daml-0.2.2.vsix
```

For another VS Code-compatible editor, use its **Install from VSIX** command and select the generated file. The declared VS Code engine baseline is `^1.85.0`. This project does not publish the extension to a marketplace.

## Executable setting and workspace trust

The extension uses `daml-lsp` from `PATH` by default. If it is elsewhere, set `definitivelyAssertedDaml.serverPath` to its absolute path in **user settings**. Workspace settings cannot substitute an executable. The extension launches the executable directly with no shell and no arguments.

The server does not start while the workspace is untrusted. Trusting the workspace starts the client; VS Code does not support revoking trust as a workspace transition. Changing the executable setting in trusted state restarts the client. A missing executable produces an error naming the configured command and pointing to release installation options.

Startup has a 30-second deadline covering executable acquisition and
initialization. Configuration replacement or deactivation can cancel an
unfinished startup without waiting for the server to respond. Context requests
do not block those lifecycle transitions.

Retirement closes outgoing work before shutdown. Pending editor callbacks and
dependency events cannot send work to the old connection or migrate it to a
replacement. Crash replacements retain the standard client's bounded restart
policy, but every restart passes through the same session owner. A failed or
unknown retirement is reported rather than treated as an idle client. If
cleanup is unknown, reload the editor instead of expecting another command to
start a replacement.

## Select authoring context

With a DAML document active, use the Command Palette:

- **DAML: Use Configuration Context** — discover the nearest ancestor `daml.bundle` from this file, or choose a local bundle folder. Optionally enter an expected native root type; leave blank to use the source's root assertion.
- **DAML: Use Definition Context** — compile this saved local file as a native definition in its selected/discovered bundle.
- **DAML: Use Syntax Only** — clear semantic context and dependency watchers.

Untitled configuration requires a selected bundle folder. Untitled definitions must be saved first. A context-selection failure preserves the prior server context; closing/reopening or restarting clears selection. Semantic hints do not insert defaults or validate incomplete buffers.

The client watches actual consulted dependencies, including local imports outside the workspace. Unsaved definition and manifest buffers override disk. Shared watcher memberships are released on context reset, close and disposal; failed replacement preserves previous memberships and reports an error. Filesystem watcher limitations, protocol commands and resource bounds are documented in the [LSP guide](https://github.com/dprb-work/daml/blob/main/docs/lsp.md#semantic-context).

Automation may invoke `definitivelyAssertedDaml.useConfiguration` with `{uri, bundleRoot?, expected?}`, `definitivelyAssertedDaml.useDefinition` with `{uri, bundleRoot?}`, or `definitivelyAssertedDaml.useSyntax` with `{uri}`. Arguments use URI strings and require an already open DAML document in a trusted workspace.

## Package maintenance

`npm run package` derives the VSIX name from the manifest version and writes
`definitively-asserted-daml-0.2.2.vsix` plus its checksum. It stages the canonical
root `LICENSE` and `SOURCES.md`, records source/dirty-state provenance in
`BUILD.json`, and generates dependency notices from the esbuild runtime graph.
These files are generated, ignored inputs—not maintained legal copies.
Move any previous same-version candidate before packaging; it is not silently
overwritten. CI receives the exact output path from the producer rather than
pinning a second filename in its installed-host checks. See
[release preparation](../../docs/development/releases.md) for approved delivery.

Dependencies are pinned in `package.json` and `package-lock.json`. Use
`npm run typecheck` for strict TypeScript checking against the declared VS Code
1.85 API baseline. Packaging does not publish to a marketplace.

The build stages the pinned language client's upstream `terminateProcess.sh` beside the bundled JavaScript, with executable permission. Keep that runtime asset when changing bundlers: the standard client uses it to terminate lingering server process trees on Linux/macOS after shutdown.

`npm run prepare:client` applies the reviewed, exact-version lifecycle
corrections in `patches/client-lifecycle.json`. Build and typecheck call it
explicitly after script-disabled installation; no `postinstall` or runtime
patching is used. Source drift fails before correction writes. Restore the
locked graph with `npm ci --ignore-scripts` instead of editing `node_modules`.
The [source inventory](../../SOURCES.md#editor-integration-dependencies) records
ownership, licenses and removal conditions.

The corrections make initialization failure terminal for that attempt; DAML
does not configure the upstream initialization-retry callback or retain its
unowned retry-dialog continuations. A deliberate new configuration can start a
new attempt after known cleanup. This is separate from the retained bounded
crash-restart policy.

The grammar is editor presentation only. It follows the [native authoring contract](https://github.com/dprb-work/daml/blob/main/docs/native-authoring.md) and [syntax contract](https://github.com/dprb-work/daml/blob/main/docs/syntax.md); it is not a parser or semantic validator. Language behavior comes from the server.

### Temporary build-tool security exception

The manifest scopes an MSAL Node `7.1.0` override to VSCE `4.0.0` through Azure Identity `4.14.0`, fixing [GHSA-xcx6-jx8p-x3h5](https://github.com/advisories/GHSA-xcx6-jx8p-x3h5). This is an explicitly accepted compatibility exception: Identity still declares MSAL `^6.0.0`; no released parent currently admits the patched version.

The [MSAL v7 migration guide](https://github.com/AzureAD/microsoft-authentication-library-for-js/blob/msal-node-v7.1.0/lib/msal-node/docs/v7-migration.md) identifies removal of interactive `responseMode`; the inspected Identity calls do not supply it. Static API review and packaging/editor checks do not establish every Azure credential or authenticated publishing flow. The high-severity npm audit remains enforced without exclusions.

Keep the parent-version selectors exact. Remove this exception when a supported parent release admits patched MSAL rather than widening it to unreviewed parents.
