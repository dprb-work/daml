# Install and format your first DAML file

Use this guide to try the language and native tools without building Rust or
Python. Start with [data versus schema](data-and-schema.md) if the distinction
between values and requirements is new to you. To complete and validate
configuration against definitions afterward, continue to [the tutorial](tutorial.md).

## 1. Install the native tools

Download the archive and matching checksum for your platform from the
[native v0.1.0 release](https://github.com/dprb-work/daml/releases/tag/v0.1.0).
Follow [the installation guide](distribution.md) to verify, extract, and add its
`bin` directory to `PATH`.

Check both executables:

```sh
daml-fmt --help
daml-lsp --help
```

The archive contains the formatter and language server. The VS Code extension is
a separate package, and neither executable is a configuration-validation CLI.

## 2. Write a file

Save this as `service.daml`:

```daml
!app.Service {
name: "demo"
adapter: !providers.Http { endpoint: "https://example.com" }
}
```

Braces define the object boundaries. The `name` and `adapter` fields belong to
the service; `endpoint` belongs to the adapter. Indentation is presentation.
The tags name definitions that a consumer can later load. Formatting does not
require those definitions.

## 3. Format it

```sh
daml-fmt service.daml
```

The command prints this to standard output and leaves `service.daml` unchanged:

```daml
!app.Service {
  name: "demo"
  adapter: !providers.Http { endpoint: "https://example.com" }
}
```

Save the output to a different file, then check it:

```sh
daml-fmt service.daml > service.formatted.daml
daml-fmt --check service.formatted.daml
```

The check exits successfully without output. On unformatted input, `--check`
exits with status 1; syntax or command errors exit with status 2. Do not redirect
the output over the input path: the shell would truncate it before reading.

## 4. Use an editor

Follow [the VS Code guide](../integrations/vscode/README.md) to build and install
the extension VSIX. Configure its server path if `daml-lsp` is not on `PATH`.
Open `service.daml` in a trusted workspace and run **Format Document**.

Try removing the final `}` to see a syntax diagnostic, then restore it. Current
editor support includes highlighting, syntax diagnostics, and document formatting.
It does not load your definitions or report schema-validation errors. For another
LSP-capable editor, see [the language-server setup](lsp.md).

## Next: definitions and defaults

[The configuration tutorial](tutorial.md) uses checked-in files and a source-built
Rust example consumer. It adds definitions, inserts defaults, accepts independent
providers, and demonstrates a validation failure.

For compact syntax lookup use [the syntax reference](syntax.md). For the reasons
behind these choices use [design principles](design-principles.md).
