## Why

Tools can edit DAML semantic values, but canonical writing discards the surrounding authored presentation. Issue [#34](https://github.com/dprb-work/daml/issues/34) needs targeted configuration edits that preserve unrelated source bytes and never materialize omitted defaults as a side effect.

## What Changes

- Add a bounded Rust source-editing API for replace, insert and remove operations at explicit RFC 6901 paths in one parsed configuration document.
- Resolve every operation against the same original snapshot. Apply a batch atomically, preserving bytes outside its declared replacement/removal/insertion regions.
- Reuse `SemanticNode`, the existing canonical semantic renderer, parser spans and schema admission. Retain exact numbers, intrinsic temporal/cron values and representable assertion bindings.
- Make comment ownership explicit: comments outside an edited subtree stay; comments inside an explicitly replaced or removed subtree belong to that subtree. Removing an entry does not consume neighboring or trailing comments.
- Reparse every successful result. A schema-checked operation also resolves the result against the caller's selected contract, but returns authored source rather than completed values.
- Bind prepared byte edits to the original source and reject a different current snapshot before returning an applied result.

## Capabilities

### New Capabilities

- `source-edits`: atomic, snapshot-bound configuration edits with explicit source-fidelity and comment-ownership guarantees.

### Modified Capabilities

None. Existing formatting, canonical writing, resolution, CLI and LSP contracts remain unchanged.

## Impact

The portable Rust library owns editing and source admission. Existing parser/source spans and semantic rendering remain authoritative; no second grammar, evaluator or filesystem writer is introduced. The proposed initial surface does not add CLI commands, binding exports, editor commands, native-definition refactoring, YAML conversion or host activation. Those integrations need their own accepted caller contracts.

This proposal changes no existing API or persisted format. The broad #34 outcome is already accepted; the detailed operation, comment and snapshot contracts below remain **pending human scope acceptance**. No implementation or result acceptance is claimed.
