## Why

Configuration authors need recurring calendar patterns, including month-end and nth-weekday rules. The user selected Spring cron to obtain those features in a recognizable existing dialect rather than invent a DAML-specific subset.

## What Changes

- Add the first-class `!Cron "..."` value and native `!Cron` kind constraint, using Spring Framework 7.0.10 cron expression admission as the reference.
- Support its six seconds-first fields, basic operators, calendar-relative operators, names, wildcard aliases and documented macros. Do not substitute Quartz, Kubernetes or a hybrid grammar.
- Retain intrinsic cron identity through existing temporal value, default, semantic/editing, staged and binding paths. Plain JSON deliberately projects the expression to a string.
- Expose immutable Python Cron values and browser kind-tagged expression text. Update standalone examples, editor classification and maintained references.
- Preserve existing date/time APIs and schema metadata. Do not add a scheduler, occurrence evaluator, timezone/DST policy, external cron-library adapters or new Python custom schema hooks.

## Capabilities

### New Capabilities
- `cron-values`: Spring-compatible first-class cron patterns and their complete value lifecycle.

### Modified Capabilities
- None. Existing temporal and Python capability contracts remain compatible; the new recurrence kind reuses their implementation owners.

## Impact

Core temporal admission, parser diagnostics, native constraints/defaults, semantic/staged value paths; Python and browser admission/export declarations; maintained docs/examples and editor token classification. No runtime Java or Spring dependency. Reference-oracle artifacts are verification inputs, not production code.

## Scope decision

The accountable user selected this direction with “ok. lets go spring” after discussion of `!Cron`, complete Spring syntax, calendar-relative rules, compatibility and no local dialect. This records that decision; it does not grant merge, release or deployment authority. The implementation contract and exclusions are in `design.md` and the capability delta.
