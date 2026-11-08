## Why

Native Regex is valuable for visually grouped, commented patterns, but its separate raw single-line scanner cannot consume DAML's multiline forms. The user approved a consolidation pass and keeping flags inline in the pattern. Sharing mechanics must not change raw Regex backslashes or ordinary string decoding.

## What Changes

- Consolidate quoted boundaries and multiline layout under the existing `src/strings.rs` owner; remove the native Regex scanner loop.
- Add native `!Regex` triple-double and triple-single payloads with shared delimiter, dedent, folding and line-ending mechanics, preserving raw Regex text.
- Migrate native pattern storage, compilation, source origins and formatting to carry admitted borrowed/owned text and its original literal range.
- Keep inline flags such as `(?x)` explicit. Do not automatically enable verbose mode or add flags metadata.
- Update affected documentation, runnable patterns and editor grammar; retain ordinary decoded strings, typed temporal values and single-line Regex behavior.

## Capabilities

### New Capabilities
- `regex-literals`: raw native Regex quoted forms, shared layout and complete compilation/formatting behavior.

### Modified Capabilities
- None. Existing ordinary string and temporal contracts remain compatible.

## Impact

Shared string scanner and literal renderer, native parser/model/compiler/formatter, editor classification and behavioral checks. No new runtime dependency or binding wire kind. This change is independent of the still-open Spring Cron PR27 and does not merge or modify it.

## Scope decision

The accountable user approved this after the raw multiline proposal: “yeah, sounds like a consolidation pass. that flag pos is fine with me”. The accepted flag position is inline in the pattern. This records scope approval, not result acceptance, merge or release authorization.
