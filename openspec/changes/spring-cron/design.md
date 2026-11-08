## Authority and compatibility reference

The user prioritized established adoption, calendar-relative features and avoiding a local dialect, then selected Spring. Use [Spring Framework 7.0.10 CronExpression](https://docs.spring.io/spring-framework/docs/7.0.10/javadoc-api/org/springframework/scheduling/support/CronExpression.html) and its [pinned source tree](https://github.com/spring-projects/spring-framework/tree/v7.0.10/spring-context/src) as the grammar reference. Independent oracle observations resolve source-level admission details; a handwritten subset of convenient operators would not satisfy the choice.

No scheduler, next-run/matches API, timezone inference or DST/catch-up engine is added. The calendar meanings are Spring's, but DAML validates and transports a pattern rather than calculating occurrences. Java is verification-only.

## One existing scalar lifecycle

Extend the existing `TemporalKind` with Cron and reuse `TemporalValue`, source-bound `ParsedTemporal`, resolved/semantic values and staged summaries. A recurrence pattern is another temporal scalar, not an ISO clock label. One cohesive cron admission module owns its grammar; do not create a parallel parsed/resolved/default hierarchy or a generic plugin registry.

The shared public Rust path is `TemporalValue::parse(TemporalKind::Cron, text)`, with existing borrowing/ownership accessors. Cron has no offset and rejects native offset facets. The parsed source-allocation identity check remains mandatory for source-backed and decoded/canonicalized payloads. Existing source ownership is established before parsing, after input bounds are checked.

Admitted text follows Spring's trimmed expression/macro expansion. Other accepted expression spelling remains retained. Equality compares this resolved expression text, not occurrence sets; the ISO fraction comparator does not interpret cron text. This avoids adding an evaluator solely to normalize or compare configuration values. Source formatting remains separate and preserves authored spelling.

## Schema and host boundaries

Reuse `x-daml-scalar` with kind cron and `x-daml-default-temporals` with kind cron. These existing temporal metadata boundaries already preserve non-JSON scalar identity and preset provenance. Renaming them would break external schema consumers without serving the new behavior. Plain JSON projects to strings; semantic wire retains `{kind: "cron", value: expression}`.

Python exposes frozen `Cron(expression)` with read-only `.expression`, string/repr and core value equality. No standard-library Cron type exists; no new Pydantic hooks or third-party adapters are inferred. Existing native schema, explicit JSON metadata and custom-exporter boundaries remain available. Browser declarations and runtime admission/export change together, with no independent parser.

## Delivery and evidence ownership

Core author owns core production; binding author owns Python/browser production. An independent check owner authors/freezes inputs, reference corpus and runner registration before production mutation. Main records valid failing-before evidence and releases authors, owns specification/docs/integration, then runs focused and aggregate checks and actual runtime scenarios. Hashes and role rules on a shared filesystem do not provide hostile-implementer write isolation.

Independent whole-change review follows actual-path verification. The source handoff is a pull request; result acceptance, merge and release require their own user authority. The already merged temporal change is archived as history in this branch.
