# Hardening baseline — 2026-10-18

This record covers library and harness sources at
`bda8882e73f658448e762fd0d64ca11f811a72d3`. Subsequent documentation and
smoke-count validation changes leave the measured Rust/build inputs unchanged.
These are bounded observations, not proof that every input is safe or that
every grammar/validator branch was reached.

## Fuzz campaigns

[Raw campaign statistics](hardening/fuzz-2026-10-18.json) record the exact bounds,
tool versions, input counts, and outcomes. Each target first passed 1,000 smoke
executions from the tracked corpus, then continued from that mutable corpus for
`-max_total_time=180` with seed 42 and AddressSanitizer. All three campaigns
exited successfully; there was no crash, timeout, or RSS-limit finding to
minimize. No production parser/resolver change was needed in this pass.

| Target | Executed inputs | Initial input files | Peak libFuzzer RSS (MB) |
|---|---:|---:|---:|
| parse | 3,289,531 | 120 | 469 |
| schema | 225,122 | 111 | 515 |
| resolve | 1,191,727 | 86 | 494 |

Reproduction command for each target:

```sh
taskset -c 0 env CARGO_BUILD_JOBS=1 TOKIO_WORKER_THREADS=1 \
  ./scripts/fuzz TARGET -max_total_time=180 -verbosity=0 -print_final_stats=1
```

The fuzz package pins nightly-2026-10-18, cargo-fuzz 0.13.2, and libfuzzer-sys
0.4.13. Tracked corpus bytes are fixed by the source revision. Mutable corpora
are ignored; initial campaign file/byte counts include the preceding smoke.
LibFuzzer scheduling and a wall-time stop do not promise identical execution
counts on another machine. `cov` counters are not a grammar coverage percentage.

Retained regressions check implicit-YAML rejection, invalid trailing input,
escaped slash decoding, punctuation as string data, and a cumulative alternative
completion budget that does not poison the next call. Existing exact-limit,
Unicode/span, schema-reference, ambiguity, and default-provenance tests remain
in the full gate.

## Release time and memory

[Raw measurements](hardening/measurements-2026-10-18.json) contain all 120 fresh
worker samples and their environment. The suite used 1,000 timed operations per
worker, three warmups, and five process repetitions. It ran after all campaigns
and builds used for fuzzing, pinned to CPU 1 on the shared Linux host.

```sh
taskset -c 1 env CARGO_BUILD_JOBS=1 TOKIO_WORKER_THREADS=1 \
  ./scripts/measure suite 1000 5
```

Compiler: Rust 1.99.1 (`58cf8f9c3`, LLVM 23.1.1), default release profile,
no RUSTFLAGS overrides, debug assertions disabled. Host: Linux
6.8.0-139-generic, x86_64, Intel Xeon E3-1275 v5 @ 3.60 GHz.

Each cell reports the median followed by the observed minimum–maximum across
five samples. Config size is record count; string size is decoded bytes; deep
size is tagged mapping depth. Peak RSS includes setup and is not incremental
per-operation memory. Parse includes source copying and decoding; resolve
excludes parse/schema compilation and borrows unchanged string payloads.

| Case | Size | Operation | Time µs/op (range) | Process peak MiB (range) |
|---|---:|---|---:|---:|
| config | 4 | parse | 3.29 (3.07–4.87) | 3.73 (3.53–3.82) |
| config | 4 | resolve | 25.54 (19.37–56.52) | 8.07 (8.04–8.23) |
| config | 32 | parse | 29.77 (25.45–53.17) | 3.70 (3.68–3.84) |
| config | 32 | resolve | 199.30 (169.26–236.32) | 8.09 (8.08–8.25) |
| config | 256 | parse | 280.23 (231.52–317.39) | 3.89 (3.74–3.96) |
| config | 256 | resolve | 1610.81 (1478.69–1898.39) | 8.61 (8.54–8.74) |
| literal | 1024 | parse | 70.30 (53.37–75.96) | 3.57 (3.49–3.61) |
| literal | 1024 | resolve | 0.83 (0.80–1.06) | 8.14 (8.05–8.18) |
| literal | 16384 | parse | 1033.18 (627.98–1262.15) | 3.56 (3.52–3.59) |
| literal | 16384 | resolve | 0.97 (0.94–13.14) | 8.26 (8.22–8.30) |
| literal | 262144 | parse | 10816.00 (10381.51–11573.99) | 4.54 (4.42–4.66) |
| literal | 262144 | resolve | 0.89 (0.85–1.08) | 9.41 (9.30–9.44) |
| wrapped | 1024 | parse | 17.96 (15.25–37.33) | 3.76 (3.55–3.78) |
| wrapped | 1024 | resolve | 4.20 (3.72–7.94) | 8.17 (8.11–8.23) |
| wrapped | 16384 | parse | 371.31 (319.40–396.02) | 3.78 (3.71–3.81) |
| wrapped | 16384 | resolve | 3.18 (3.00–3.28) | 8.21 (8.17–8.25) |
| wrapped | 262144 | parse | 7226.74 (6276.42–8484.39) | 4.56 (4.43–4.56) |
| wrapped | 262144 | resolve | 3.00 (2.86–30.07) | 9.23 (9.12–9.26) |
| deep | 4 | parse | 2.18 (2.15–2.31) | 3.68 (3.61–3.79) |
| deep | 4 | resolve | 12.73 (10.64–37.75) | 8.19 (8.11–8.23) |
| deep | 16 | parse | 13.81 (11.17–37.11) | 3.75 (3.58–3.81) |
| deep | 16 | resolve | 73.90 (46.83–77.29) | 8.18 (8.12–8.26) |
| deep | 64 | parse | 54.31 (37.85–70.42) | 3.75 (3.63–3.82) |
| deep | 64 | resolve | 355.53 (325.90–483.32) | 8.28 (8.24–8.32) |

The host shows substantial timing noise, including resolver outliers; the raw
ranges must accompany comparisons. There is no CI timing threshold. Inspection
of `dedent_and_join` and `wrap_lines` found bounded linear passes with one owned
output when transformation is needed, not a copy per physical line. The
resolver's `check_node` charges subtree work for each assertion: deeply tagged
chains revisit descendant work and are not promised linear scaling. That
conservative accounting remains bounded by depth and the cumulative work limit.
The measurements do not justify a new cache, unsafe code, or a different AST.

## Verification and distribution

`./scripts/check` passed: library/fuzz formatting, all-target library Clippy,
113 integration tests, two doctests, and warning-clean rustdoc. Standalone fuzz
Clippy passed. Actual `inspect`, `resolve`, and `schema_adapter` consumers ran;
resolution produced the default timeout 30 and request ID `local`. All 24
release worker variants passed their decoded value/tag/default checks before
the recorded suite. CLI invalid-target/run-count/size paths returned errors.

Review found a developer-wrapper defect: a smoke count above signed `int`
range can disable libFuzzer's run limit. The pre-fix invocation
`scripts/fuzz-smoke 2147483648` launched a target and required a three-second
external timeout. The repaired wrapper rejects that count, arbitrarily large
counts, zero, leading zeroes, empty input, and nondigits with exit status 2
before Cargo. Normal bounded target execution remains the real smoke check.
This is a tooling finding, not a parser campaign finding. The numeric CLI
boundary was exercised directly; no permanent test launches a potentially
unlimited campaign if that guard regresses.

Both locked dependency-policy checks passed. Duplicate transitive versions
remain reviewed warnings (`syn`, plus fuzz-only `getrandom`/`r-efi`); the library
graph also reports the intentionally unused fuzz-only license exception.
Dependency IDs, edges, and unified features reachable from DAML match across
both lockfiles (87 dependencies); the fuzz runtime does not alter the library
graph. YAML fixture blob IDs and full MIT notice are retained in
[SOURCES.md](../../SOURCES.md). No upstream parser code was copied.

Initial unconstrained build attempts failed to allocate compiler/tool worker
threads. CPU affinity and one Cargo build job allowed the subsequent checks,
ASan builds, campaigns, and measurements to finish. These setup failures were
not DAML input findings and are excluded from the measurement samples.

Both packages remain unpublished. DAML's own license, final key/tag alphabets,
formatter, LSP, and Python packaging are separate decisions/work, not claims of
this baseline. See the [hardening guide](hardening.md) for the maintained commands.
