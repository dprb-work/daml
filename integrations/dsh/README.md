# DSH DAML language server

Configure the native Soul DSH LSP extension with the released `daml-lsp` executable. The extension owns process lifecycle and document synchronization; DAML owns syntax diagnostics and formatting.

Prerequisite: Soul's native client must include `lsp_format` from
`runtime/dsh/extensions/src/lsp.ts`. Build that canonical package with
`npm --prefix runtime/dsh/extensions run build` in a prepared Soul checkout;
if using the assembled Soul bundle, rebuild it through Soul's documented build
procedure as well. This DAML configuration payload neither ships a second DSH
client nor upgrades or activates an installed host. Older clients can provide
syntax diagnostics but do not acquire formatting merely by adding this entry.

The complete native extension configuration template is `lsp.json` in this directory:

```json
{
  "servers": [
    {
      "language": "daml",
      "command": "daml-lsp",
      "args": []
    }
  ]
}
```

Merge this entry into the existing `servers` array and preserve every other configured server; do not replace a complete host configuration. `daml-lsp` must be installed and available on `PATH`. If it is not, set `command` to the absolute path of the installed executable. The config template does not install binaries, modify active host configuration, or enable the extension. `language` is sent as the LSP `textDocument.languageId`.

The native DSH `lsp_format` tool is a read-only preview. It returns the workspace URI, the opaque workspace file-version token for the exact text snapshot used, and UTF-16 LSP text edits. A consumer applying edits must validate that token with the workspace's guarded write/edit path; the tool never writes. Formatting uses DAML's fixed formatter style, explicitly requests UTF-16 positions, and fails clearly if the server does not advertise document formatting.
