# Service configuration tutorial bundle

These are the canonical files used by [data and schema](../../docs/data-and-schema.md),
[the tutorial](../../docs/tutorial.md), and
[the structural-composition explanation](../../docs/structural-composition.md).
All endpoints are illustrative strings; the example consumer performs no network
request.

| Location | Role |
|---|---|
| `daml.bundle` | Entry context selects the app and provider bundles |
| `app/Worker.daml` | Introductory schema: required fields plus a retry preset |
| `data/worker.daml` | A particular worker configuration, not a schema |
| `invalid/retries.daml` | Deliberate schema failure for a supplied string retry count |
| `app/Service.daml` | Consumer requires a name, structural adapter, and retry count |
| `contracts/Adapter.daml` | Open structural requirement: a string endpoint |
| `providers/Http.daml` | Independent closed record with a timeout default |
| `third-party/Custom.daml` | Independent closed record with a label default |
| `data/` | Valid configuration and string examples |
| `invalid/` | Deliberate validation failures |

Each definition directory owns a `daml.bundle`. The app imports `contracts` in
its own manifest. Neither provider imports that contract: compatibility is
established by the completed value satisfying both its own assertion and the
containing field's requirements.

Run from the repository root:

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/worker.daml
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/configured.daml
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/provider.daml
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/external.daml
```

The HTTP provider receives `timeout: 30`; the custom provider receives
`label: "external"`. Both service configurations receive `retries: 3`.

The worker receives `retries: 3`. Supplying `retries: "many"` in
`invalid/retries.daml` must fail at `/retries`; the preset cannot repair an
invalid supplied value. [The two-layer explanation](../../docs/data-and-schema.md)
shows the definition, data, result, and failure together.

The following command must fail with a schema violation at
`/adapter/endpoint`, because its value is a number rather than a string:

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/invalid/endpoint.daml
```
