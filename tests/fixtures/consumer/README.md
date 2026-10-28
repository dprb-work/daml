# Derived consumer regression fixtures

These files retain a historical consumer-schema conversion for regression
coverage. They are test inputs, not the introductory DAML example.
See [source provenance](../../../SOURCES.md#representative-consumer-definitions)
for their origin and [the tutorial](../../../docs/tutorial.md) for a
self-contained introduction.

Run from the repository root:

```sh
cargo test --locked --test consumer_fixtures
```

## What was converted

| Native definition | Source data contract retained |
|---|---|
| `olaf/config/Recipe.daml` | Setup ID string/object alternatives, task/process mode, params, optional adapter/step/service/logger/tracer collections, optional execution, nullable optimization |
| `olaf/config/ExecutionConfig.daml` | Continue/fail-fast policy and fail-fast default |
| `olaf/config/OptimizeBudget.daml` | Integer candidate/concurrency values and concurrency default 1 |
| `olaf/config/OptimizeConfig.daml` | Required objective, walker and budget; optional lineage and writers |
| `questdb/json_query.daml` | Required string query, optional adapter object, timestamp-filter default, nullable mixed string/integer unpack list |
| `questdb/adapter.daml`, `redis/adapter.daml` | Open object data contracts selected through tags, without marker fields |

The recipe's adapters use an open structural `Adapter` contract, not an
implementation collector. Each provider can use a different private alias for
the same protocol bundle. The root binds `olaf`, `questdb`, and `redis`; it cannot
use the recipe bundle's private `api` alias. One file defines one type.

`Adapter` and `Step` assert only an object shape because the sampled callable
schemas supplied no further standard data constraints. They do not prove that a
Python object implements a runtime API. Untagged compatible objects are valid;
tags select additional validation contracts, not constructors.

## Deliberate differences and limits

- Tags replace `__type__` and `__fn__` data markers. No nominal implementation map
  or central closed-world provider union is retained.
- Callable `parameters`/`returns`, dispatch aliases, import paths, and construction
  remain consumer responsibilities. `setup_id` preserves its object alternative
  as a data shape; DAML does not establish that it is callable.
- Missing members retain the sampled schema's optionality. Explicit partial
  objects receive defaults; for example, `execution: {}` receives
  `actor_error_mode: "fail_fast"`.
- The integer budget contract does not invent positivity constraints absent from
  the source. This proof is not an audit of Olaf runtime policy.
- The source's overlapping marker enum inside the nested optimizer budget is not
  copied into a native discriminator. The shared budget definition validates the
  same ordinary integer fields.
- JSON Schema `default` alone is an annotation. The example deliberately uses
  DAML completion to apply those defaults; plain JSON Schema validation does not
  have equivalent completion behavior.

The acceptance cases check completed data and nested tags, reject five invalid
value shapes at their data paths, accept an untagged structural adapter, and
reject a private alias from the root. They do not certify the entire Olaf schema
corpus, migrate any Olaf configuration, or exercise Olaf execution.
