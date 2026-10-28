# Integration verification

The verification package covers the VS Code extension host and DAML grammar.
Local VS Code checks use the released `daml-lsp`; editor CI builds the locked
server for the checked-out revision.

## VS Code extension host (Linux/Xvfb)

Prerequisites: Node.js, npm, `xvfb-run`, and the released `daml-lsp` executable.
From the DAML repository root:

```sh
cd integrations/vscode
npm ci
npm run package
cd ../verify
npm ci
DAML_LSP_PATH=/absolute/path/to/released/bin/daml-lsp npm run test:vscode
```

The runner downloads pinned VS Code 1.105.1, installs the generated VSIX in
isolated extension and user-data directories, then starts two real Xvfb-backed
Electron extension hosts. The trusted case checks the language ID, Unicode
diagnostics and clearing, formatting and no-op behavior, invalid-input
non-destructiveness, and process cleanup after a client restart. It also checks
that a hostile workspace `serverPath` sentinel remains unused. The untrusted
case checks `workspace.isTrusted` before and after opening a DAML document and
that its user-scoped sentinel remains unused. No mock LSP is used.

If the Electron check fails, the runner prints wrapper and test-driver events
plus recorded process IDs before cleaning its scratch directory. The diagnostic
does not capture or alter language-server stdio.

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
builds `daml-lsp` with the locked Rust dependency graph, then runs the installed
VSIX in Xvfb-backed Electron and retains the VSIX artifact for 14 days. See the
[CI path map](../../docs/development/ci.md) for all selected boundaries.
