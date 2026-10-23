# Integration verification

Local host probes use the immutable DAML v0.1.0 release; editor CI builds the locked `daml-lsp` for the checked-out revision so LSP-source changes are covered. These checks exercise installable integration boundaries and do not themselves accept the production source.

## VS Code extension host (Linux/Xvfb)

Prerequisites: Node.js, npm, `xvfb-run`, and the released `daml-lsp` executable. From the DAML repository root:

```sh
cd integrations/vscode
npm ci
npm run package
cd ../verify
npm ci
DAML_LSP_PATH=/absolute/path/to/released/bin/daml-lsp npm run test:vscode
```

The TextMate boundary check runs the actual `vscode-textmate` and Oniguruma engines against the extension grammar. It checks escaped backslash/quote handling, triple-string body/comment boundaries, and `!Regex` backslash/comment scopes:

```sh
npm run test:grammar
```

`DAML_GRAMMAR_PATH` optionally selects a grammar JSON for a before/after regression reproduction; normal CI uses the checked-out grammar.

The runner downloads pinned VS Code 1.105.1, installs the generated VSIX in isolated extension/user-data directories, then starts two real Xvfb-backed Electron extension hosts. The trusted case verifies language ID, Unicode diagnostics/clearing, formatting/no-op behavior, invalid-input non-destructiveness, and process cleanup on client restart: the configured wrapper runs the released server with inherited stdio, remains alive after its child exits, and must be terminated by the client's Unix fallback after a user-setting change. Its PID and child PID are checked for exit; the runner kills only these task-owned leftovers on failure. A hostile workspace `serverPath` sentinel must remain unused. The `@vscode/test-electron` runner forces trusted mode, so the untrusted case launches Electron directly with the extension test driver and asserts `workspace.isTrusted === false` before and after opening a DAML document; its user-scoped sentinel must remain unused after process exit. The trusted configuration supplies the released executable to the wrapper and updates the user setting to the released binary to trigger restart. Local runs use the released server's directory prepended to `PATH`; CI builds the locked server for the checked-out revision. There is no mock LSP. Cursor is not launched or claimed as runtime-verified.

If the Electron check fails, the runner prints wrapper start/spawn/child-exit events, test-driver stages, and recorded wrapper/child PID files before cleaning the scratch directory; this diagnostic does not capture or alter language-server stdio.

For a persistent rendered editor surface and a CDP endpoint for Main's screenshot check, build the VSIX as above and launch:

```sh
DAML_VSCODE_NO_SANDBOX=1 \
DAML_LSP_PATH=/absolute/path/to/released/bin/daml-lsp \
VSCODE_CDP_PORT=9222 npm run test:vscode:visual
```

The launcher prints its PID, `http://127.0.0.1:9222` CDP endpoint, and open `.daml` file, then keeps the actual Electron editor open until it exits or receives SIGINT/SIGTERM. This is a real VS Code surface; Cursor is not claimed. The installer uses `resolveCliArgsFromVSCodeExecutablePath` to invoke VS Code's CLI rather than starting the Electron binary in install mode.

`DAML_VSCODE_NO_SANDBOX=1` is a process-local opt-in for environments where Electron's setuid sandbox cannot run (for example, some unprivileged Linux VMs). It weakens Chromium's process isolation: use only with the scratch workspace, no private files/secrets, and a trusted disposable verification machine. CI leaves this variable unset; don't modify host sandbox ownership or permissions.

## OMP packaged-registration process

Prerequisites: Linux/POSIX Python 3, the installed `omp` executable, and the released `daml-lsp` plus sibling `daml-fmt`. From the DAML repository root:

```sh
OMP_BINARY=/absolute/path/to/omp \
DAML_LSP_PATH=/absolute/path/to/released/bin/daml-lsp \
DAML_OMP_EVIDENCE=/absolute/new/evidence-directory \
python3 integrations/verify/omp/run.py
```

This launches the actual OMP RPC process and its built-in LSP tool in isolated HOME/config/data/cache/temp directories, without sending a model prompt. It loads the repository's `integrations/omp/lsp.json` payload into a project `.omp/lsp.json` merge that also contains an unrelated existing registration, using the released server executable. The reused `skills/verify-daml-distribution/scripts/omp-probe.mjs` asserts actual host negotiation, Unicode diagnostics, replacement/clearing, formatting, edit application, idempotence, and formatter acceptance. Evidence is retained under the requested new directory; isolated runtime state is removed on success.

## Native DSH formatting preview

Prerequisites: the Soul checkout with its declared DSH dependencies available (including Cordis and `@deepseek-ai/dsh-fs-local`), Bun, and the released `daml-lsp` executable. From the Soul repository root:

```sh
DAML_LSP_PATH=/absolute/path/to/released/bin/daml-lsp \
bun test runtime/dsh/extensions/test/lsp-format.test.ts
```

The focused test uses the canonical native `LspTools` client and a real released-server subprocess. A temporary controlled workspace adapter checks the returned `{ uri, version, edits }` (`version` is its opaque FsVersion), exact Unicode UTF-16 whole-document edit, canonical-input no-op, and no writes. It also creates a controlled FsVersion ABA: a real `lsp_diagnostics` call synchronizes different text to the same LSP URI while formatting awaits its snapshot guard; the formatting preview must reject `STALE_FILE`. If `DAML_LSP_PATH` is absent the test is explicitly skipped during unrelated suite discovery; the focused command above sets it, and a skip is not acceptance.

For actual Cordis tool execution with the canonical native filesystem and workspace adapter, run this isolated DSH composition smoke:

```sh
SOUL_REPO_PATH=/absolute/path/to/soul \
DAML_LSP_PATH=/absolute/path/to/released/bin/daml-lsp \
bun /absolute/path/to/daml/integrations/verify/dsh/smoke.mjs
```

It mounts Cordis `Context`, native `LocalFileSystem`, the actual `ToolRuntime`, and Soul's canonical `NativeWorkspaceAccess`, then invokes `lsp_diagnostics` on invalid DAML, replaces it and checks diagnostic clearing, previews formatting, confirms no write, and checks canonical formatting returns no edits. It uses no model or mocked LSP process. The focused Bun regression isolates snapshot interleaving; this smoke exercises composition but is not a complete deployed DSH-host acceptance.

For a deployed DSH Cordis configuration, mount the host adapter after DSH's native scoped filesystem, observation policy, user approval, and tools plugins, then mount the LSP export with a merged server list (preserving every existing entry):


```yaml
- name: "@soul/dsh-extensions/host"
  config:
    workspaceRoot: /absolute/workspace/root
- name: "@soul/dsh-extensions/lsp"
  config:
    servers:
      - language: typescript
        command: typescript-language-server
        args: ["--stdio"]
      - language: daml
        command: /absolute/path/to/daml-lsp
        args: []
```

The host export provides `dshExtensionWorkspace` backed by the deployed DSH filesystem and its authorization policy; do not provide a second ad hoc filesystem or apply returned text edits as raw writes. `lsp_format` is a preview: the consumer applies `edits` only after confirming the returned opaque `FsVersion` still identifies the workspace snapshot it read, and routes any subsequent write through DSH's existing authorized path. `integrations/dsh/lsp.json` is the installable DAML server entry to merge into that list.
## Editor CI

`.github/workflows/editors.yml` runs on pull requests and `main` only when editor verification, VSIX source, or DAML Rust sources change. It installs both separate npm lockfiles with scripts disabled, audits dependencies, typechecks and packages the extension, builds `daml-lsp` with the locked Rust dependency graph, then runs the installed VSIX in Xvfb-backed Electron. It retains the private VSIX as a 14-day Actions artifact. OMP's proprietary RPC host and the cross-repository native DSH package remain explicit local probes; neither is represented by an editor CI substitute.
