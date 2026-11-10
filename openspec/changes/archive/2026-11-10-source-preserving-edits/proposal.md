## Why

Tools can edit DAML semantic values, but canonical writing discards the surrounding authored presentation. Issue [#34](https://github.com/dprb-work/daml/issues/34) needs targeted configuration edits that preserve unrelated source bytes and never materialize omitted defaults as a side effect.

## What Changes

- Add a bounded Rust source-editing API for replace, insert and remove operations at explicit RFC 6901 paths in one parsed configuration document, with `daml edit` as its first usable caller.
- Resolve every operation against the same original snapshot. Apply a batch atomically, preserving bytes outside its declared replacement/removal/insertion regions.
- Reuse `SemanticNode`, the existing canonical semantic renderer, parser spans and schema admission. Retain exact numbers, intrinsic temporal/cron values and representable assertion bindings.
- Make comment ownership explicit: comments outside an edited subtree stay; comments inside an explicitly replaced or removed subtree belong to that subtree. Removing an entry does not consume neighboring or trailing comments.
- Reparse every successful result. A schema-checked operation also resolves the result against the caller's selected contract, but returns authored source rather than completed values.
- Bind prepared byte edits to the original source and reject a different current snapshot before returning an applied result.

## Capabilities

### New Capabilities

- `source-edits`: atomic, snapshot-bound configuration edits with explicit source-fidelity and comment-ownership guarantees.

### Modified Capabilities

- `configuration-cli`: add an explicit edit-batch input and authored-source output while retaining existing schema/context selection, bounded admission, exit statuses and read-only file behavior.

## Impact

The portable Rust library owns editing and source admission. Existing parser/source spans and semantic rendering remain authoritative; no second grammar, evaluator or filesystem writer is introduced. The CLI reads a bounded DAML batch through `--edits FILE`, applies the core patch, validates the edited document using existing bundle/module/expected context, and prints authored source without materializing omitted defaults. LSP commands, binding exports, native-definition refactoring, YAML conversion and host activation remain out of scope.

The owner [accepted the operation/comment/snapshot rules and selected the CLI caller](https://github.com/dprb-work/daml/pull/41#issuecomment-6777065470). This proposal changes no existing API or persisted format. Result acceptance, merge, release and activation remain separate; no completed implementation is claimed here.
