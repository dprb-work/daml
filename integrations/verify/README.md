# Integration verification

The verification package covers the VS Code extension host and DAML grammar.
Local and CI editor checks use a source-built `daml-lsp` matching the checked-out
extension. Older released executables may lack the semantic providers.

## VS Code extension host (Linux/Xvfb)

Prerequisites: Node.js, npm, `xvfb-run`, Rust/Cargo, and a matching source-built server.
From the DAML repository root:

```sh
cargo build --locked --features lsp --bin daml-lsp
cd integrations/vscode
npm ci
npm run package
cd ../verify
npm ci
DAML_LSP_PATH=/absolute/path/to/checkout/target/debug/daml-lsp \
  DAML_VSIX_PATH=/absolute/path/to/definitively-asserted-daml-0.2.0.vsix \
  npm run test:vscode
```

On an isolated headless host without a usable Chromium sandbox helper, prefix the
test command with `DAML_VSCODE_NO_SANDBOX=1`. This existing runner option disables
Electron's process sandbox for the disposable test profile, not VS Code workspace
trust. Do not use it for a normal editor session.

The runner downloads pinned VS Code 1.105.1, installs the generated VSIX in
isolated extension and user-data directories. The aggregate runs lifecycle and
semantic suites in real Xvfb-backed Electron extension hosts. The trusted lifecycle case checks the language ID, Unicode
diagnostics and clearing, formatting and no-op behavior, invalid-input
non-destructiveness, and process cleanup after a client restart. It also checks
that a hostile workspace `serverPath` sentinel remains unused. The untrusted
case checks `workspace.isTrusted` before and after opening a DAML document and
that its user-scoped sentinel remains unused. No mock LSP is used.

Both the installed-host and visual runners require that exact absolute VSIX
path. They do not choose a latest file or import a product manifest to infer
which archive the caller intended to test.

If the Electron check fails, the runner prints wrapper and test-driver events
plus recorded process IDs before cleaning its scratch directory. The diagnostic
does not capture or alter language-server stdio.

The semantic suite checks explicit configuration/definition/syntax commands,
incomplete field/name completion, checked hover/default provenance, real source
navigation, unsaved definitions/manifests, Unicode positions and external-file
watch refresh after actual dependency-buffer closure. It uses repository-owned
fixtures and an isolated profile, not mocked providers. Run just this suite with
`npm run test:vscode:semantic`, or lifecycle only with `npm run test:vscode:lifecycle`,
using the same absolute `DAML_LSP_PATH`.

### Watcher readiness and tracing

Context selection is not a backend-readiness barrier. The pinned host registers
[extension-host filters synchronously](https://github.com/microsoft/vscode/blob/1.105.1/src/vs/workbench/api/common/extHostFileSystemEventService.ts),
but [initializes native watching asynchronously](https://github.com/microsoft/vscode/blob/1.105.1/src/vs/platform/files/node/watcher/nodejs/nodejsWatcherLib.ts)
before attaching its change listener. The public watcher has no readiness promise.

For file-link retarget probes, seed all inputs before Code launches. Reuse the
external providers folder after an actual Http error/recovery roundtrip, and keep
that Service selection alive to retain the established folder watch. During the
measured transition, change only the selected link entry; no manifest, target,
configuration, or provider-request activity may conceal a missing link watch.

To inspect protocol, watcher startup, and bounded retarget action timestamps,
prefix the semantic command with `DAML_VERIFY_TRACE=1`. It is off by default.
Tracing changes scheduling; a traced pass is not proof that the default suite
passes.

## Grammar

Run the TextMate boundary check against the extension grammar:

```sh
cd integrations/verify
npm ci
npm run test:grammar
```

The check uses the actual `vscode-textmate` and Oniguruma engines. It covers
escaped backslash and quote handling, triple-string body/comment boundaries,
and `!Regex` backslash/comment scopes. Set `DAML_GRAMMAR_PATH` to select a
grammar JSON for a before/after regression reproduction; normal CI uses the
checked-out grammar.

## Editor CI

The automatic `.github/workflows/check.yml` planner selects the reusable editor
suite for integration sources, language-server protocol sources, and shared
build dependencies; draft pull requests skip it. `.github/workflows/editors.yml`
remains a manual entrypoint. The heavy workflow installs both npm lockfiles with
scripts disabled, audits dependencies, typechecks and packages the extension,
builds `daml-lsp` with the locked Rust dependency graph, then runs both installed
VSIX host suites in Xvfb-backed Electron and retains the VSIX artifact for 14 days. See the
[CI path map](../../docs/development/ci.md) for all selected boundaries.
