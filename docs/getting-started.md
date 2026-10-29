# Install and format your first DAML file

Want to get a feel for DAML before building anything? Start with a file you can
format and edit. You'll see how the structure reads and what the editor catches;
[the tutorial](tutorial.md) adds definitions, defaults, and validation afterward.

## 1. Get the native tools

Download your platform's archive and matching checksum from the
[native v0.1.0 release](https://github.com/dprb-work/daml/releases/tag/v0.1.0).
[The installation guide](distribution.md) walks through verification, extraction,
and adding the archive's `bin` directory to `PATH`.

Check that both tools are available:

```sh
daml-fmt --help
daml-lsp --help
```

They run without Rust or Python. `daml-fmt` formats files; `daml-lsp` supports
editors. The VS Code extension is a separate package, and neither native command
is a configuration-validation CLI.

## 2. Write something to format

In a scratch directory, save this as `service.daml`:

```daml
!app.Service {
name: "demo"
adapter: !providers.Http { endpoint: "https://example.com" }
}
```

The service has a name and an adapter. Its braces already say which fields belong
together, even though the indentation isn't helpful yet. The tags select
definitions when a consumer validates the data; you can format the file before
loading those definitions.

## 3. Let the formatter make it readable

```sh
daml-fmt service.daml
```

It prints this and leaves your source file alone:

```daml
!app.Service {
  name: "demo"
  adapter: !providers.Http { endpoint: "https://example.com" }
}
```

Save the output to a different file and check it:

```sh
daml-fmt service.daml > service.formatted.daml
daml-fmt --check service.formatted.daml
```

The check succeeds silently. On the original unformatted input, `--check` would
exit with status 1; syntax or command errors exit with status 2. Don't redirect
over the input path: the shell would truncate it before the formatter reads it.

## 4. Try the editor feedback

Follow [the VS Code guide](../integrations/vscode/README.md) to build and install
the extension VSIX. Set its server path if `daml-lsp` isn't on `PATH`, open your
file in a trusted workspace, and run **Format Document**.

Remove the final `}` and notice the syntax diagnostic; then put it back. That is
the current editor experience: highlighting, syntax diagnostics, and formatting.
Checking a value against its definitions is a separate operation, not something
the language server does today. For another LSP-capable editor, use
[the server guide](lsp.md).

## Next: give the file some requirements

[Build a service configuration](tutorial.md) to see a retry default appear, a bad
value rejected, and an independent provider selected. It uses checked-in examples
and a source-built Rust consumer. If you want the distinction between a
configuration and its requirements first, read [data and schema](data-and-schema.md).
Use [syntax](syntax.md) for lookup and [design principles](design-principles.md)
for the reasons behind the choices.
