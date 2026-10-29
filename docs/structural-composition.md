# Structural composition

You've configured an HTTP adapter. Later, another package offers a different
provider. Do you have to teach the service's schema the name of every provider
it might ever accept?

Not if the service describes what it needs rather than listing who may supply
it. That is the useful idea behind DAML's structural contracts. Start with
[data and schema](data-and-schema.md) if values and requirements are still new;
[the tutorial](tutorial.md) runs the examples below.

## Ask for what the service needs

The service's `adapter` field refers to `contracts.Adapter`. That definition,
[`Adapter.daml`](../examples/tutorial/contracts/Adapter.daml), asks for a string
endpoint:

```daml
!Object {
  endpoint: !String
}
```

It doesn't ask which package defined the value. It also doesn't forbid settings
that a particular provider needs in addition to the endpoint. The contract is
open because it has no `closed` modifier.

[`Service.daml`](../examples/tutorial/app/Service.daml) uses that contract for
its adapter field. Its own record is closed: extra service fields are errors.
That doesn't close the nested adapter. The two definitions answer different
questions at different levels of the configuration.

## Let each provider describe its own settings

[`Http.daml`](../examples/tutorial/providers/Http.daml) adds a timeout:

```daml
closed !Object {
  endpoint: !String
  timeout: !Int
} = {
  timeout: 30
}
```

It doesn't reference `contracts.Adapter` or declare that it implements it.
A completed HTTP value fits because it has the required string endpoint. Its
extra timeout field is allowed by the service's open adapter contract.

[`Custom.daml`](../examples/tutorial/third-party/Custom.daml) makes a different
choice:

```daml
closed !Object {
  endpoint: !String
  label: !String
} = {
  label: "external"
}
```

That can fit too. Its completed value has the endpoint the service requires,
plus a provider-specific label. Both provider records are closed, yet both can
satisfy the open `Adapter` contract. Closing a provider's own fields is not the
same as closing the world of compatible providers.

This is an **open-world structural protocol**: another provider can meet the
contract without joining a central list. DAML has no separate `protocol`
declaration; an ordinary structural definition plays that role.

## Make a provider available, then choose it

The entry's [`daml.bundle`](../examples/tutorial/daml.bundle) makes names
available through imports:

```daml
{
  app: "./app"
  providers: "./providers"
  extra: "./third-party"
}
```

The data chooses a provider:

```daml
!app.Service {
  name: "demo"
  adapter: !providers.Http { endpoint: "https://example.com" }
}
```

The [other version](../examples/tutorial/data/external.daml) uses `!extra.Custom`.
Changing the provider means making its bundle available and selecting its tag.
It does not require adding that provider's name to `Service.daml` or
`Adapter.daml`.

Imports are local. The app bundle has its own `contracts` binding; that name
isn't exposed at the entry just because `app` imports it. See
[bundle scope](native-authoring.md#files-and-bundle-roots) for exact lookup rules.

## A provider choice doesn't waive the requirements

Selecting `!providers.Http` means the value must satisfy the HTTP definition
**and** the service's adapter requirement. A known tag isn't a pass around
validation.

An endpoint of `42` fails the string requirement. A supplied timeout of `"slow"`
fails the HTTP definition. Omitting the timeout is different: its default supplies
`30`. And an extra field forbidden by a closed provider record still fails even
though the outer adapter contract is open.

The same adapter can be untagged and still meet the service's structural
requirement. A tag is useful when you want that additional definition, including
its constraints and defaults; it isn't proof of a nominal ancestry relationship.

## What about inheritance and closed alternatives?

If you're used to nominal types, compatibility often comes from declared
membership or ancestry. DAML definition references reuse constraints and defaults;
they don't introduce that kind of membership. Inheritance is not inherently
closed-world in languages that support it, either.

Keep three questions apart:

- **Does this value fit?** Structural compatibility checks its fields and values.
- **May it have more fields?** Record openness decides that for each constraint.
- **Which alternatives do we intend to accept?** A consumer may deliberately
  restrict the accepted shapes rather than admit anything meeting one shared
  minimum contract.

For the last choice, a definition can use a union:

```daml
!providers.Http | !extra.Custom
```

That lists alternative **shapes**, not permitted provider identities. Another
tagged definition can still pass if its value satisfies one branch. Adding a
shape outside both branches requires changing the union; adding a new provider
that already fits does not. Exact provider or tag allowlisting is a separate
consumer policy.

Use a shared structural contract when independent providers should qualify by
meeting common requirements. Use alternatives when the shapes themselves are the
choices you intend to support. The [native reference](native-authoring.md#composition-records-and-positions)
defines composition and record rules; [schema interchange](schemas.md#compose-independently-owned-definitions)
describes the low-level API path.
