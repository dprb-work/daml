# Structural composition

Start with [data and schema](data-and-schema.md) if the distinction between
a supplied value and its requirements is new to you. This page builds on that
distinction: independent definitions can constrain the same value.

DAML checks whether a value satisfies its constraints. It does not require a consumer to declare that a provider implements a named protocol. This is an open-world structural protocol: the contract is the required shape and constraints, and an independently authored definition can satisfy it without ancestry or a membership declaration.

Follow the [tutorial](tutorial.md) for the runnable example. The [native authoring contract](native-authoring.md#composition-records-and-positions) owns definition syntax and record rules; [schema validation](schemas.md#compose-independently-owned-definitions) describes the interchange path.

## A consumer defines the required shape

The canonical example uses `examples/tutorial/app/Service.daml` to require a contract named `Adapter`:

```daml
closed !Object {
  name: !String
  adapter: !contracts.Adapter
  retries: !Int
}
```

`examples/tutorial/contracts/Adapter.daml` defines the adapter contract:

```daml
!Object {
  endpoint: !String
}
```

Here `Adapter` is open because it has no `closed` modifier. A value may contain additional fields and still satisfy the adapter's requirements. The `Service` record itself is closed, independently of the openness of the nested adapter.

## Independent implementations can satisfy the contract

`examples/tutorial/providers/Http.daml` is an independent provider definition:

```daml
closed !Object {
  endpoint: !String
  timeout: !Int
} = {
  timeout: 30
}
```

It does not reference or inherit from `contracts.Adapter`. Its records are structurally compatible: the provider requires an endpoint, permits the required service value to carry `timeout`, and supplies a default timeout. `examples/tutorial/third-party/Custom.daml` is another independent definition:

```daml
closed !Object {
  endpoint: !String
  label: !String
} = {
  label: "external"
}
```

Both providers have their own closed records. Closing a provider's record does not stop its value from satisfying the open `Adapter` constraint: the extra fields are allowed by the parent constraint. `examples/tutorial/data/provider.daml` and `external.daml` show these choices in the full service configuration.

The consumer's entry manifest binds aliases to bundle directories. To make a provider selectable at an entry, add or change an alias in that entry's `daml.bundle`, for example:

```daml
{
  app: "./app"
  providers: "./providers"
  extra: "./third-party"
}
```

Adding a provider changes the entry's import table and the tag spelling available through that binding, such as `!providers.Http` or `!extra.Custom`. It does not change the `Service` definition or its `adapter` contract. A consumer can select an independently authored provider as the adapter value without editing the consumer's contract. Bundle bindings are local to each bundle; see [bundle roots and bindings](native-authoring.md#files-and-bundle-roots). The `contracts` binding belongs to the nested app bundle, not this entry manifest.

## Keep three axes separate

Structural compatibility and record openness answer different questions. Nominal systems add declared identity or ancestry as another compatibility rule; DAML references reuse constraints structurally and do not introduce nominal membership.

- **Structural compatibility** asks whether a value satisfies all applicable constraints. The parent contract and an explicit tag's selected definition both constrain the value.
- **Record openness** controls whether a record constraint permits undeclared fields. Records are open by default; `closed` rejects additional fields for that constraint, including fields not declared by a referenced base.

Inheritance is not inherently closed-world in systems that have inheritance. A derived record may remain open unless its constraints close it. In DAML, referencing or narrowing a definition does not by itself create nominal ancestry. An unrelated closed record can satisfy an open parent structurally. See [composition and record modifiers](native-authoring.md#composition-records-and-positions).

## Explicit tags add assertions, not escape hatches

An untagged adapter value is checked against the `Adapter` constraint. A tagged value such as `!providers.Http` is checked against both the parent `Adapter` constraint and the explicitly selected `Http` definition. DAML does not ignore either constraint because the other passed.

If a value violates either constraint, resolution fails. For example, a missing or non-string `endpoint` violates the parent contract even if the provider tag is recognized. A string supplied for `timeout` violates the provider definition; when `timeout` is omitted, the provider's default supplies `30`. A field forbidden by a closed constraint also fails. Tags therefore preserve a useful explicit assertion without turning definitions into a mandatory protocol registry.

## Use an enumerated union when the set is closed by design

Some consumers intentionally accept only a fixed set of alternatives. Express that choice with a union:

```daml
!providers.Http | !extra.Custom
```

This is an alternative between structural shapes, not an allowlist of provider identities. A value tagged with another definition can still pass if it satisfies a branch's constraints. Adding a provider whose shape fits an existing branch does not require changing the union; a shape outside all branches does. True provider or tag allowlisting is a separate consumer policy. Use a union when the listed shapes are the intended alternatives; use a shared structural requirement when independent implementations should qualify by satisfying one contract. DAML's union completion and validation rules are described in [composition and defaults](native-authoring.md#composition-records-and-positions).
