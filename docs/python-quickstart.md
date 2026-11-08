# Configure a Python program with DAML

Your configuration types describe what your program needs. A DAML file supplies
one concrete set of values. In this tutorial, you derive a schema from a Python
model, read a configuration file, and see both default completion and validation.

The Python distribution is named `daml-config`; your code imports `daml`.
It is not published to PyPI yet. Use a verified candidate wheel if you have one,
or build from the current source checkout. The native `0.1.0` archives do not
contain these Python bindings.

## 1. Install the Python bindings

### Candidate wheel: no Rust build

Follow [Install a candidate wheel](python.md#install-a-candidate-wheel) to obtain
the wheel and its checksum, create an environment, and install the `types` extra.
Keep that environment active and use a working directory of your choice for the
files below. You don't need a source checkout.

### Source build

Install Git, [Rust through rustup](https://rustup.rs/), and
[uv](https://docs.astral.sh/uv/getting-started/installation/). Clone DAML and enter
the checkout, or use the root of an existing current checkout:

```sh
git clone https://github.com/dprb-work/daml.git
cd daml
uv sync --project bindings/python --locked --python 3.12 --extra types --reinstall-package daml-config
```

uv provisions the Python 3.12 environment; the checkout pins Rust. Activate the
environment with `. bindings/python/.venv/bin/activate` on Linux/macOS, or
`.\bindings\python\.venv\Scripts\Activate.ps1` in PowerShell.
The remaining commands use the active environment's `python`.

## 2. Describe a worker in code

Save this as `read_worker.py` in your working directory:

```python
from pathlib import Path

import daml
from pydantic import BaseModel


class Worker(BaseModel):
    name: str
    retries: int = 3


module = daml.schema_from_types({"app.Worker": Worker})
schemas = daml.Schemas.compose([module])
document = daml.parse(Path("worker.daml").read_text(encoding="utf-8"))

try:
    resolved = schemas.resolve(document, expected="app.Worker")
except daml.ResolveError as error:
    print(f"Invalid configuration at {error.path}")
    raise SystemExit(1) from None

print(resolved.to_python())
```

`schema_from_types` derives an in-memory schema module from the model. The
mapping gives the contract its DAML name, `app.Worker`; it need not match the
Python class name. `expected="app.Worker"` requires the root value to meet that
contract.

## 3. Supply the values in DAML

Save this as `worker.daml` beside the script:

```daml
!app.Worker {
  name: "demo"
}
```

The `!app.Worker` assertion selects the named contract. The file supplies a
worker's name and leaves the retry count to its declared default.

Run the consumer beside `worker.daml`:

```sh
python read_worker.py
```

It prints:

```text
{'name': 'demo', 'retries': 3}
```

The schema is like a type: many configurations can satisfy it. This document is
like a value: one worker with particular settings. Resolving the document fills
in missing defaults and checks the result. The returned object is a Python
dictionary, not a `Worker` instance.

## 4. Override a default

Replace `worker.daml` with:

```daml
!app.Worker {
  name: "demo"
  retries: 5
}
```

Run the same command again. It prints:

```text
{'name': 'demo', 'retries': 5}
```

The supplied value wins over the default. You don't need to repeat every setting
in each configuration file.

## 5. Try a value that doesn't fit

Change the retry count to a string:

```daml
!app.Worker {
  name: "demo"
  retries: "many"
}
```

Run the consumer again. It prints this message and exits with status 1:

```text
Invalid configuration at /retries
```

The file is valid DAML syntax, but its value doesn't satisfy the schema.
A default fills an omission; it doesn't replace an invalid supplied value.
The script displays the failing path. The caught `ResolveError` also carries
the detailed diagnostic; see [Python errors](python.md#errors).

## 6. Use a dataclass instead

Replace the Pydantic import and the `Worker` definition in `read_worker.py` with:

```python
from dataclasses import dataclass


@dataclass
class Worker:
    name: str
    retries: int = 3
```

Keep `import daml`, the `Path` import, and the rest of the consumer. Restore the
DAML file from step 3 and run the same command. It prints the same completed
values. Standard dataclasses use the same schema-module and resolution APIs.

Neither path runs model construction or promises Pydantic's coercion rules.
Schema derivation supports declarative fields, supported constraints, and static
defaults; custom validators and default factories are rejected rather than
silently omitted. The [Python API reference](python.md#derive-schemas-from-python-types)
describes the supported subset.

## Where to go next

- If you used a source checkout, run the checked-in
  [Pydantic and dataclass example](../examples/python_types.py) from its root:

  ```sh
  python examples/python_types.py
  ```

- Follow the [native configuration tutorial](tutorial.md) to write schemas in
  DAML and accept independently authored providers through a shared structural
  contract. You don't need Python for that workflow.
- Read [data and schema](data-and-schema.md) for the type/value analogy, or the
  [YAML/JSON comparison](comparison.md) for the tradeoffs.
- [Set up the formatter and editor](getting-started.md). Current editor support
  provides syntax diagnostics and formatting, not schema-aware validation.
