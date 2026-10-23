# Editor and host integrations

These clients wrap the existing `daml-lsp`; they do not load bundles, complete defaults, or add schema-aware editor features. Install the native executable from the private [v0.1.0 release](https://github.com/dprb-work/daml/releases/tag/v0.1.0) separately. Native archives remain independent of editor runtimes.

| Client | Delivery | Installation |
|---|---|---|
| VS Code / Cursor | Private installable VSIX; highlighting, syntax diagnostics, document formatting | [Build and install](vscode/README.md) |
| OMP | Project `.omp/lsp.json` registration payload | [Merge registration](omp/README.md) |
| Native DSH / Soul | Native LSP server-list entry; generic formatting preview in Soul's canonical client | [Merge registration and client prerequisite](dsh/README.md) |

## Build the VSIX

From this repository:

```sh
cd integrations/vscode
npm ci --ignore-scripts
npm run typecheck
npm run package
```

Install `definitively-asserted-daml-0.1.0.vsix` with the editor's **Extensions: Install from VSIX...** command. The package uses language id `definitively-asserted-daml`, distinct from Digital Asset's unrelated DAML. The native server defaults to `daml-lsp` on `PATH`; a machine-scoped user setting accepts an absolute executable path. It is never downloaded or launched through a shell by the extension. Server execution requires workspace trust. Packaging stages the canonical root MIT license and generates notices for the dependencies actually bundled.

Host JSON files are configuration payloads, not installers. Preserve existing server registrations when merging them. No user-wide host settings, installed runtime links, startup files, or repository visibility are changed by this delivery.

## DSH ownership

The native DSH transport and tools belong to Soul's `runtime/dsh/extensions/`, not this repository. Its read-only `lsp_format` capability returns a URI, native filesystem freshness token, and UTF-16 edits; it never applies edits. A consumer must retain the returned `FsVersion` when routing a later write through the host's existing authorization path. Concurrent same-document synchronization invalidates the preview. Building this source does not activate an installed Soul/DSH host.

## Verification and limits

[Verification commands](verify/README.md) exercise the installed VSIX in a real VS Code 1.105.1 Electron extension host, the packaged OMP registration in real OMP 18.12.6 RPC, and native DSH tools composed with Cordis, the real local filesystem, and Soul's workspace adapter. The local editor scenario uses the released Linux executable. The editor CI workflow builds the candidate server and retains a private VSIX artifact for 14 days.

Trusted and restricted Electron scenarios verify that a hostile workspace
`serverPath` cannot replace the machine/user executable and that an untrusted
workspace retains highlighting without launching its configured server.
Actual TextMate/Oniguruma checks cover escaped-backslash closing delimiters,
escaped quotes, triple-string comment boundaries, and raw Regex contents.
The escaped-backslash case fails against the original grammar and passes after
the fix. Native DSH's same-document snapshot regression likewise fails before
its guard and passes after; its strict package build and Soul's full
`pnpm run check` also pass.

Review regressions also cover cold-start overlap: document generations are
reserved before yielding to notification writes, so preview B cannot receive
edits for earlier text A with B's filesystem token. The real-server case fails
before reservation and passes after. The VSIX retains the standard client's
Unix termination helper; a real released server in a lingering parent process
survives restart without the helper and is terminated with it. The native
composition smoke removes its owned workspace through unconditional cleanup.

A live VS Code renderer was also driven through a Unicode syntax error, valid replacement, and **Format Document**; diagnostics cleared and the canonical Unicode document rendered correctly. Cursor uses the same VSIX/API contract but was not launched here. Interactive OMP TUI and a complete deployed DSH session are not claimed. On the local verification VM, Electron required an explicit process-only `DAML_VSCODE_NO_SANDBOX=1`; no host sandbox privileges were changed.

No marketplace/registry publication, active-host activation, YAML/JSON conversion, Helm integration, or richer language-server semantics are included.
