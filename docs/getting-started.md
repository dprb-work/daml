# Install and format your first DAML file

This guide gets you editing a DAML file without building Rust or Python.
To see configuration defaults and validation first, follow the
[Python quickstart](python-quickstart.md). The [native tutorial](tutorial.md)
shows handwritten definitions and independent providers.

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
editors. The VS Code extension is a separate package. To validate configuration,
build the separate [`daml` CLI](cli.md) from source.

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

Remove the final `}` and notice the syntax diagnostic; then put it back.
Highlighting, syntax diagnostics and formatting work without selecting schemas.
For semantic validation, completion and navigation, build the current matching
server/client and [select an authoring context](../integrations/vscode/README.md#select-authoring-context).
The released server used above may predate these features. For another
LSP-capable editor, use [the server guide](lsp.md).

## Next: give the file some requirements

[Configure a Python program](python-quickstart.md) to derive requirements from a
Pydantic model or dataclass, or [build a service configuration](tutorial.md)
with native DAML definitions. Both paths show a default appear and a bad value
rejected. Read [data and schema](data-and-schema.md) for the distinction between
values and their requirements.
Use [syntax](syntax.md) for lookup and [design principles](design-principles.md)
for the reasons behind the choices.
