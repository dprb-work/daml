# Build a service configuration

Imagine a service whose configuration starts small: a name, a retry count, and
later a choice of adapter. You want sensible defaults, but you also want the
choices people do supply to be checked.

Let's build that up. You'll first run a worker configuration, change its retry
count, and see a useful failure. Then you'll give a service an HTTP adapter and
swap in a separately defined provider without rewriting the service's contract.
The examples print configuration values; the sample endpoints are not contacted.

## Before you start

Use a source checkout with [the pinned Rust toolchain](development/building.md)
installed, and run commands from the repository root. Cargo builds the existing
Rust example consumer, which completes and validates configuration and prints
JSON. You don't need Python or the VS Code extension for this tutorial.

If you only want to try editing first, use [the formatter guide](getting-started.md).
The released native executables provide formatting and LSP support; the
resolution steps below use the source-built library consumer.

The files under [`examples/tutorial/`](../examples/tutorial/README.md) show
handwritten DAML definitions and native resolution. A separate
[Python types example](../examples/python_types.py) derives in-memory schema
modules from Pydantic models and dataclasses; it does not generate native
`.daml` definition files.

## 1. Give the worker a name

Open [`data/worker.daml`](../examples/tutorial/data/worker.daml):

```daml
!app.Worker {
  name: "demo"
}
```

It names the configuration's definition, `app.Worker`, and supplies a name. Run it:

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/worker.daml
```

You get more than the file supplied:

```json
{
  "name": "demo",
  "retries": 3
}
```

Where did three retries come from? Look at
[`app/Worker.daml`](../examples/tutorial/app/Worker.daml):

```daml
closed !Object {
  name: !String
  retries: !Int
} = {
  retries: 3
}
```

This is the **schema layer**. It requires a string name and an integer retry
count; the data after `=` supplies a default of three. `closed` means unexpected
worker fields are errors rather than silently accepted additions.

The configuration you ran is the **data layer**: one particular worker. DAML
fills its missing retry count, then checks the completed value against the
requirements. It doesn't rewrite the input file.

You can read the definition as a type and the configuration as a value if that
analogy is familiar. Otherwise, *requirements* and *supplied contents* are enough.
[Data and schema](data-and-schema.md) explores both ways of thinking about it.

## 2. Make a choice, then make a mistake

Suppose this worker needs five retries. Save your own copy as `worker.daml` in
the repository root, using another filename if that one is already in use:

```daml
!app.Worker {
  name: "demo"
  retries: 5
}
```

Pass your copy to the same consumer, substituting its filename if needed:

```sh
cargo run --locked --example native -- examples/tutorial worker.daml
```

The result contains `"retries": 5`. The definition supplies a baseline; your
explicit value takes precedence.

Now change just the retry value in your copy:

```daml
!app.Worker {
  name: "demo"
  retries: "many"
}
```

Run the same command again. It must fail with a schema violation at `/retries`.
`"many"` is valid string syntax, but it isn't an integer. DAML doesn't replace a
bad supplied choice with a default and carry on.

Restore `retries: 5` if you want to keep using your copy. The remaining steps use
the checked-in examples, so you don't need to edit them. You can also reproduce
this failure directly with [`invalid/retries.daml`](../examples/tutorial/invalid/retries.daml).

## 3. Give the service an adapter

A worker's fields are straightforward. What happens when a configuration needs
another configurable component?

[`data/configured.daml`](../examples/tutorial/data/configured.daml) supplies an
adapter with an endpoint:

```daml
!app.Service {
  name: "demo"
  adapter: { endpoint: "https://example.com" }
}
```

Run it:

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/configured.daml
```

The service also receives `"retries": 3`; its adapter keeps the endpoint you
supplied. Here is the service's definition in
[`app/Service.daml`](../examples/tutorial/app/Service.daml):

```daml
closed !Object {
  name: !String
  adapter: !contracts.Adapter
  retries: !Int
} = {
  retries: 3
}
```

Instead of naming every possible provider, it asks for an `Adapter`.
[`contracts/Adapter.daml`](../examples/tutorial/contracts/Adapter.daml) says what
that means:

```daml
!Object {
  endpoint: !String
}
```

An adapter must have a string endpoint. This record is open, so it can also have
settings the service doesn't need to know about. The service's own record is
closed; that doesn't make every nested record closed too.

## 4. Choose HTTP and let it supply its settings

The HTTP provider has its own definition in
[`providers/Http.daml`](../examples/tutorial/providers/Http.daml):

```daml
closed !Object {
  endpoint: !String
  timeout: !Int
} = {
  timeout: 30
}
```

Select it on the adapter value, as
[`data/provider.daml`](../examples/tutorial/data/provider.daml) does:

```daml
!app.Service {
  name: "demo"
  adapter: !providers.Http { endpoint: "https://example.com" }
}
```

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/provider.daml
```

The result now contains both defaults:

```json
{
  "adapter": {
    "endpoint": "https://example.com",
    "timeout": 30
  },
  "name": "demo",
  "retries": 3
}
```

The service supplies its retry default. The HTTP definition supplies its timeout.
Choosing HTTP doesn't waive the service's requirements: the endpoint still has
to be a string, and any supplied timeout still has to be an integer.

## 5. Swap providers, not the service contract

A separately authored definition,
[`third-party/Custom.daml`](../examples/tutorial/third-party/Custom.daml), also
requires a string endpoint. It has a label instead of an HTTP timeout and supplies
`label: "external"` as its default.

[`data/external.daml`](../examples/tutorial/data/external.daml) selects it with
`!extra.Custom`. Run that version:

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/external.daml
```

The adapter now contains `"label": "external"`; the service still gets three
retries. Neither `Service.daml` nor `Adapter.daml` needed a new branch listing
that provider. Its completed value meets the same requirement.

The [`daml.bundle` files](../examples/tutorial/daml.bundle) make those names
available. The entry bundle binds `app`, `providers`, and `extra` to directories;
the app bundle has its own `contracts` binding. To expose another provider, the
entry context needs a binding for its bundle. That is a wiring change, not a
change to the service's requirements.

This is the practical benefit of structural compatibility. Read
[structural composition](structural-composition.md) for open-world providers,
closed alternatives, and why record openness is a separate question.

One final check: a numeric endpoint doesn't meet the contract, whether or not
other fields look plausible. The deliberately invalid example demonstrates it:

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/invalid/endpoint.daml
```

Expect a schema violation at `/adapter/endpoint`.

## Consume it from a program

The command you've been running is a small Rust consumer. Here is the same path
with fixed input filenames, ready to read as a complete `main` function:

```rust
use daml::Bundle;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string("examples/tutorial/data/provider.daml")?;
    let document = daml::parse(source)?;
    let bundle = Bundle::open("examples/tutorial")?;
    let schemas = bundle.compile(&document, None)?;
    let resolved = schemas.resolve(&document)?;
    println!("{}", serde_json::to_string_pretty(&resolved.to_json())?);
    Ok(())
}
```

[`examples/native.rs`](../examples/native.rs) is the runnable version with
command-line arguments and explicit output-error handling. Repository examples
can use `daml` and the already-declared `serde_json` dependency. In another Rust
project, add a source/path dependency on DAML and declare `serde_json`; the DAML
crate is not published to crates.io.

`document` retains the authored source and spans. `resolved` gives you completed
values and their metadata. Printing `to_json()` exports plain values, not tags or
the original document. [Python](python.md#parse-complete-and-inspect) and
[browser JavaScript](../bindings/browser/README.md#load-and-edit) show the same
engine from other languages.

## Explore text and source

For layout-preserving text versus wrapped prose, inspect the string example:

```sh
cargo run --locked --example inspect -- examples/tutorial/data/text.daml
```

`instructions` decodes to `Read the configuration. Report any missing values.`
The literal `template` keeps `service:\n  name: demo`. To inspect plain data
without selecting a definition, run:

```sh
cargo run --locked --example inspect -- examples/tutorial/data/plain.daml
```

## Where to go next

You've supplied values, used defaults, rejected an invalid choice, and selected
independent providers. For the reasons behind those choices, read the
[design principles](design-principles.md) or [YAML/JSON comparison](comparison.md).
For exact rules, use [syntax](syntax.md), [native definitions](native-authoring.md),
and [completion and validation](schemas.md). [The examples catalogue](../examples/README.md)
links the larger corpus; [interoperability](conversion.md) explains the limits of
plain exports and the currently missing YAML/JSON converters.
