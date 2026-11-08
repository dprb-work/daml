## Why

Configuration needs real dates and times without guessing locales or timezones, or losing precision in host datetime objects. The user selected first-class temporal values, the existing `!Regex "..."`-style spelling, one Time/DateTime type with optional explicit offsets and schema requirements, and ISO 8601 as the standards basis, then authorized implementation.

## What Changes

- Add first-class `!Date "..."`, `!Time "..."` and `!DateTime "..."` literals, usable without loading schemas. Bare constructors in native definitions constrain the corresponding kinds; generic assertions remain separate from literal construction.
- Define one explicit ISO 8601-1:2019 extended-calendar profile, including the relevant 2022 technical corrections, rather than inherit whichever inputs a host parser accepts. The specification owns exact supported spellings and limits; this is not an exhaustive ISO 8601 conformance claim.
- Preserve all supplied fractional-second digits and offset presence. Fill omitted lower-order clock components with zero; never infer a machine timezone, truncate precision or normalize leap/end-of-day labels away.
- Carry temporal identity through parsed/resolved/semantic/staged values, defaults, source formatting and semantic writing. Preserve existing bounds and error/provenance behavior.
- Integrate the actual Python and browser value boundaries. Python exports dedicated immutable DAML temporal values, with explicit exact conversion to standard-library datetime types; browser transport retains temporal kind and ISO text rather than coercing through JavaScript Date.
- Extend Python type-derived schemas for standard date/time/datetime fields, representable defaults and explicit offset requirements. Keep unsupported callbacks, lossy temporal constraints and optional-dependency isolation guarded.
- Document JSON projection separately from lossless temporal transport, and provide standalone native and Python examples.
- Archive the already-integrated Python schema change and establish its canonical capability spec as lifecycle bookkeeping.

## Capabilities

### New Capabilities

- `temporal-values`: ISO-profile first-class literals, native constraints, lossless lifecycle and host/export boundaries.

### Modified Capabilities

- `python-type-schemas`: allow representable standard temporal fields and defaults without relaxing unrelated rejection policy.

## Impact

The Rust value enums, parser, native compiler/default metadata, resolver, semantic writer, validator bridge and staged summaries are affected. Python/WASM bridges, public type declarations, exhaustive consumers, editor token classification and examples must move with them. Date/Time/DateTime become reserved intrinsic literal constructors; generic assertion stacking remains rejected except an assertion applied to a temporal literal.

No cron, durations, intervals, relative/natural-language parsing, numeric epochs, locale-dependent input, named-zone/TZDB lookup, date arithmetic, model instantiation, package publication, host setup or consuming-application integration. ISO edge cases and conversion limits are explicit contracts, not silent fallbacks. The user authorized merging the previous Python feature only; publication of this feature is a reviewed-source handoff, not an automatic merge or runtime release.
