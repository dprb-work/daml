# Design

## Browser admission uses the existing interchange boundary

`Schemas.fromJsonModules(modules)` accepts a readonly array of strict JSON module strings. String input retains duplicate-member detection and exact JSON number text; ordinary JavaScript objects cannot preserve either reliably.

The bridge admits the array and strings within existing count and aggregate UTF-8 limits. It parses each envelope with `SchemaModule::from_json`, then moves the admitted modules into `Schemas::compose`. Core schema/reference/compilation limits remain authoritative. No public intermediate module-handle class or second validator is needed.

Resources remain explicit and local. The caller supplies every required URI; module admission and later operations do not discover or fetch missing resources. Foreign getter exceptions retain their original JavaScript identity.

## Validation reuses compiled constraints without completion

`Schemas::validate_semantic`, Python `Schemas.validate_semantic`, and browser `Schemas.validateSemantic` check supplied data and explicit assertions. They do not select completion candidates, invoke stages, construct containers, apply defaults or mutate caller data.

The core reuses semantic admission, expected-type lookup, tag/binding admission and compiled schema checks. Existing node, byte, depth, exact-number and evaluation-work budgets still apply.

Validation accepts literal JSON mapping keys even when DAML source cannot represent them. That is an operation boundary, not a grammar extension: ordinary semantic resolution and writing keep their source-key restrictions. Validation success does not promise successful DAML serialization.

## Recursive schema evaluation makes instance progress

The scope owner accepted bounded structural recursion after the real consumer
closure exposed recursively nested values already supported by that consumer.
Cycles that can evaluate the same instance indefinitely remain invalid. A schema
definition's containment is not itself an evaluation edge.

Schema preparation must retain finite resource and compilation bounds, propagate
default/uniqueness reachability through recursive components, and establish a
sound bound on actual recursive instance evaluation before calling the existing
validator. Completion keeps its current shared node/byte/depth/work budgets:
non-terminating required-container or default expansion returns `LimitExceeded`,
not a fabricated leaf or silently omitted value. No second validator, increased
budget, application-name special case or network lookup is introduced.

The independent check owner owns progressing object/array recursion, local and
cross-resource references, descendant failures, finite default provenance,
definition-only containment, non-progressing loop rejection, excessive recursive
branch work and non-terminating completion. Actual consumer qualification remains
separate from synthetic core/browser parity.

## One bounded compiled owner

Real consumer qualification found that per-view validator builds repeatedly
expanded the recursive graph and exceeded aggregate compilation work despite
the complete selected module fitting physical node/text limits. Keep those
limits; eliminate repeated builds instead of weakening the consumer contract.

The existing registry first resolves original static references, nested IDs and
anchors. Compile one private normalized resource with canonical view references
and one borrowed single-field dispatcher. A constant internal resource scope
allows upstream reference-target compilation to be shared. Original resources
remain the owners of constraints, binding identity, defaults and provenance;
normalization is neither a public schema format nor a second validator.

Only vocabulary-defined schema positions may be rewritten. Literal defaults,
constants, enum values, examples and extension data remain unchanged. Bound
normalized payload and actual compilation work before the engine call. Dispatch
must not copy the instance or allocate its key per validation, and internal
dispatch/schema identifiers must not replace public instance or schema locations.

## Application ownership

Consumers choose named roots and the complete reachable resource graph. They retain constructor identity, aliases, callable signatures, runtime references, overlays and application-specific numeric distinctions. The library neither imports application symbols nor interprets their discriminator fields.

A consumer whose complete metadata catalog exceeds a context budget must select the complete graph for its actual roots or fail admission. It must not truncate constraints, disable bounds, or fall back to another validator.

## Verification

Native and browser probes consume identical strict module strings with cross-module references, defaults and an exact integer above JavaScript's safe-integer range. Browser checks run offline after SDK preload. Python and browser checks distinguish shape validation from completion, enforce nested assertions and retain literal-key values without relaxing source admission.
