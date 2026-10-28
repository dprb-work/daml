# Contributor documentation

DAML is an independent experimental language and implementation. This section is for contributors working on its parser, schema engine, native definitions, tools, bindings, and distribution. It is not a prerequisite for learning or using DAML. Run documented repository commands from the repository root unless a command says otherwise.

## Build and maintain

- [Building and checks](building.md): toolchains, local commands, CI behavior, and change expectations.
- [Architecture and implementation boundaries](architecture.md): code ownership and defaults for new work.
- [Native archive distribution](distribution.md): build, install, provenance, and archive verification.

## Research and historical evidence

These records preserve the scope and evidence of earlier investigations. Their dates and stated limits matter; they are not current verification results or contribution gates.

- [Parser reuse assessment](parser-reuse.md): pinned parser references, corpus review, and reuse decision.
- [Formatter reuse assessment](formatter-reuse.md): printer references, host comparisons, and selected implementation.
- [Hardening tools](hardening.md): fuzzing, measurement, and dependency-policy commands.
- [Hardening baseline, 2026-10-18](hardening-baseline.md): recorded bounded campaigns, samples, and exact source revision.
- [Edge-case record](edge-cases.md) and [raw probe output](edge-cases.json): preserved parser observations and provenance.

See also [source and distribution notices](../../SOURCES.md).