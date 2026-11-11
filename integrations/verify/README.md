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
npm ci --ignore-scripts
npm run package
cd ../verify
npm ci --ignore-scripts
DAML_LSP_PATH=/absolute/path/to/checkout/target/debug/daml-lsp \
  DAML_VSIX_PATH=/absolute/path/to/definitively-asserted-daml-0.2.1.vsix \
  npm run test:vscode
```

On an isolated headless host without a usable Chromium sandbox helper, prefix the
test command with `DAML_VSCODE_NO_SANDBOX=1`. This existing runner option disables
Electron's process sandbox for the disposable test profile, not VS Code workspace
trust. Do not use it for a normal editor session.

The runner uses pinned VS Code 1.105.1 and isolated extension/user-data
directories. The aggregate checks RPC promise settlement, real library
startup/disposal, final writer admission, installed lifecycle and adversarial
transitions, and semantic authoring.
Generated JavaScript server wrappers use the runner's actual Node executable.
They do not rely on the editor's shell-resolved PATH retaining a Node version
manager's executable directory.

The trusted lifecycle case checks Unicode diagnostics, clearing, formatting,
invalid-input non-destructiveness and non-exec-wrapper cleanup. It records
outgoing protocol ordering independently of server tolerance and withholds a
real context response across replacement. The untrusted case keeps its server
sentinel unused; a hostile workspace executable setting remains unused in both.
The installed cases use a real native server, not a mock LSP.

Both the installed-host and visual runners require that exact absolute VSIX
path. They do not choose a latest file or import a product manifest to infer
which archive the caller intended to test.

On failure, the harness prints fixture protocol and lifecycle events before
cleaning its scratch directory. Instrumented proxies capture frames and
deliberately withhold selected responses to establish the tested interleavings.
This capture is verification-only; the shipped client does not trace document
payloads as part of its lifecycle control.

The semantic suite checks explicit configuration/definition/syntax commands,
incomplete field/name completion, checked hover/default provenance, real source
navigation, unsaved definitions/manifests, Unicode positions and external-file
watch refresh after actual dependency-buffer closure. It uses repository-owned
fixtures and an isolated profile, not mocked providers. Run just this suite with
`npm run test:vscode:semantic`, or lifecycle only with `npm run test:vscode:lifecycle`,
using the same absolute `DAML_LSP_PATH`.

### Lifecycle boundary checks

From `integrations/verify`, `npm run test:client-lifecycle` exercises the actual
installed JSON-RPC request owner with controlled asynchronous writes.
`npm run test:client-dependencies` runs the actual client library in a real
Electron extension host, including pre-spawn cancellation and cleanup failures.
`npm run test:client-writer` compiles the production writer owner and checks
bootstrap, retirement, FIFO, local refusal and pending/failed initialization
delivery without a fake editor API.

`npm run test:client-consumers` follows refusals through real registered semantic
and formatting providers, including already-submitted work. Cancellation or
inactivity must reject rather than become the SDK's successful `null` default.
It also observes actual owned channels and late process diagnostics.

`npm run test:vscode:root-consumers` checks the installed controller. Genuine
configuration events and snapshots are delivered as a controlled batch, and a
watcher-producing context reply is released while retirement is held. Its API
observer forwards real VS Code methods and resources; a reader beacon followed
by an event-loop barrier drains stale continuations before watcher assertions.

Rust terminal-budget tests separately use the already-locked channel library as
a test-only dependency. Undrained rendezvous output must time out, and ready
messages cannot override an expired deadline. Those checks do not claim
interruption of arbitrary OS writes.

`npm run test:vscode:adversarial` uses the same absolute native and VSIX paths.
It checks the actual 30-second startup deadline, repeated startup replacement,
deactivation, retained failed retirement and crash-policy history. Set
`DAML_VSCODE_ADVERSARIAL_SCENARIO` to `startup-timeout`, `startup-replacement`,
`startup-deactivation`, `startup-coalescing`, `retirement-failure` or
`crash-lineage` to select one case. A missing initialization reply cannot block
retirement, and replacement construction cannot reset the stock crash budget.
The failed-retirement fixture redirects shutdown to an unsupported native
method, so a genuine correlated protocol error—not a fabricated success—must
leave cleanup failure owned and prevent replacement.

These checks distinguish accepted message submission from remote response
completion. A passing restart against a tolerant server is insufficient if
ordinary client traffic crosses shutdown.

### Pinned host diagnostic limitation

The failed-retirement case can expose a VS Code 1.105.1 output-channel factory
warning about adding resources to an already-disposed store. An isolated probe
reproduced the same warning using only `createOutputChannel`, `appendLine` and
`dispose`, without loading DAML. The host's
[asynchronous channel registration](https://github.com/microsoft/vscode/blob/1.105.1/src/vs/workbench/api/common/extHostOutput.ts)
adds resources after the public channel can already have been disposed.
This is a host limitation, not proof of a new LSP transition failure or zero
host resource leaks. Do not suppress it or add scheduling delays to hide it;
changing the pinned host requires a separate compatibility decision.

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
builds `daml-lsp` with the locked Rust dependency graph, then runs the aggregate
dependency/writer/installed-host selection and retains the VSIX artifact for 14 days. See the
[CI path map](../../docs/development/ci.md) for all selected boundaries.
