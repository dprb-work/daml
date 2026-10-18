# Run the hardening tools

DAML's hardening tools exercise the portable parser, native schema ingestion,
and default resolver. They do not execute application constructors, import
consumer formats, or finalize the provisional key/tag alphabet.

The [recorded baseline](hardening-baseline.md) retains the bounded campaign
statistics, measurement samples, environment, and interpretation.

## Run address-sanitized fuzzing

Install the pinned development tool from the repository root:

```sh
cargo install cargo-fuzz --version 0.13.2 --locked
(cd fuzz && rustup show)
./scripts/fuzz-smoke 1000
```

The isolated `fuzz/` package pins `nightly-2026-10-18`; the library keeps its
stable toolchain. The wrapper fetches the committed fuzz dependency lock before
cargo-fuzz's internal build because cargo-fuzz has no `--locked` switch.
The smoke count must be an integer in `1..=2147483647`, without leading zeroes.
Larger values can overflow libFuzzer's signed `-runs` flag and remove its bound;
the smoke wrapper rejects them before fetching or launching a target.

Run a bounded campaign for each target:

```sh
./scripts/fuzz parse -max_total_time=180
./scripts/fuzz schema -max_total_time=180
./scripts/fuzz resolve -max_total_time=180
```

The wrapper selects AddressSanitizer, seed 42, a ten-second per-input timeout,
and a 1 GiB RSS ceiling. Parse/resolve inputs are at most 64 KiB; schema inputs
are at most 16 KiB plus their mode byte. Later libFuzzer flags override wrapper
defaults. Set `CARGO_BUILD_JOBS=1` when building on a constrained machine.

The target contracts differ:

- `parse` checks successful trees, UTF-8 byte spans, duplicate-key exclusion,
  checked slicing, and semantic stability after appending a newline.
- `schema` reads a leading `J` for JSON or `D` for DAML, then ingests and composes
  the remaining native module. Successful modules validate fixed object, array,
  scalar, null, and tagged probes through the real resolver.
- `resolve` parses DAML and applies fixed, independently composed native modules.
  It exercises tags, slots, defaults, branch ambiguity, exact numbers, uniqueness,
  resource bounds, and source/default provenance. Fixture checks run once.

Human-authored seeds live in `fuzz/seeds/`. The parse target also reads the
licensed originals and native adaptations under `tests/fixtures/yaml/`; see
[provenance and distribution boundaries](../SOURCES.md). Corpus inputs include
intentional syntax and schema errors: structured rejection is a valid outcome.
Mutable corpora, artifacts, coverage, and build outputs remain ignored.

Treat a crash as a candidate finding, not proof of a library defect. Check
whether the failed invariant belongs to DAML or to the harness. For a real
finding, minimize it with `cargo fuzz tmin` from `fuzz/`, reproduce it through
the public API, retain a consumer-visible regression, fix the cause, and rerun
the affected campaign. Do not catch panics or weaken limits to make fuzzing pass.

## Measure time and process memory

Commit measurement source and build inputs first. On Linux, run:

```sh
./scripts/measure suite 1000 5
```

The command builds the release example and emits JSON on stdout. It runs 24
case/operation combinations, each in five fresh processes with three warmups
and 1,000 timed iterations. Pin one allowed CPU with `taskset` when comparing
runs; avoid concurrent builds or fuzz campaigns.

The cases cover tagged configuration records (4/32/256), dedented literal and
wrapped strings (1/16/256 KiB decoded), and deeply tagged mappings (4/16/64).
Workload checks verify decoded values, tags, depth, and completed defaults before
timing. Parse includes the source ownership copy. Resolve parses and composes
once, then measures resolution and result destruction without schema compilation.

Each sample records elapsed time, time per operation, source/decoded size, and
Linux `VmHWM` peak RSS. RSS includes process startup, workload generation,
validation, and schema setup; it is not an allocation count or the incremental
memory used by one resolution. The suite records Git revision, source
cleanliness, compiler, build flags, CPU affinity, kernel, and timestamp.
Measurements from a shared machine are observations, not throughput guarantees
or CI thresholds. Inspect all samples rather than comparing a single minimum.

## Check both dependency graphs

```sh
./scripts/check
cargo clippy --manifest-path fuzz/Cargo.toml --all-targets --locked -- -D warnings
cargo deny --locked check
cargo deny --manifest-path fuzz/Cargo.toml --locked check
```

CI runs the stable gate, both policy graphs, and a bounded fuzz smoke. The
`libfuzzer-sys` NCSA license exception is crate-scoped. Both packages remain
unpublished until DAML's own distribution terms are selected explicitly.
