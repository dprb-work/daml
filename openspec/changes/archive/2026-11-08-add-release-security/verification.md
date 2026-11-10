# Release security pilot — verification and handoff

Scope: add-release-security. Pilot commits 074e95b, 1735ae2 and
1dfbcb3529cd30debec882feea43fa353e14d0ad were authored on feat/release-security,
based on 32ee55dcfe4447d937e6327d4c1bec6288b0c2bb. The receipts below describe
that isolated pilot, not the assembled release integration. No release
publication or provider/billing changes were performed.

## Observed pilot evidence

- 32 release/admission, inventory and Docker-identity unittest cases passed.
- Real pinned Trivy detected four advisories in an intentionally old dependency
  metadata fixture and one synthetic secret; clean secret input produced none.
- Real pinned Semgrep detected all five configured positive rules. The actual
  DAML scan covered 133 source files with zero findings.
- Real pinned ClamAV accepted a clean archive and detected the harmless EICAR
  marker inside a diagnostic archive. FreshClam refreshed and tested daily28177,
  main63 and bytecode339 without granting container capabilities.
- A complete clean-suite CLI run passed all four categories. CLI verify admitted
  matching proof and rejected payload bytes changed afterward.
- The complete DAML source suite at commit
  1735ae2221131929e23ed5790d0f6c97a287cd04 executed all categories: dependencies
  returned one finding; source secrets, SAST and artifact malware checks passed.
  Exit1 correctly blocked publication rather than declaring a clean candidate.
- All nine DAML locks had complete inventory coverage, including the legitimately
  empty browser package and UV's unversioned local editable root.

The artifact used in the suite smoke was a real benign diagnostic ZIP, not all
eight shipping producer payloads. Full hosted producer-to-retention qualification
remains owned by the existing delivery work; do not treat these receipts as a
publishable DAML candidate or authorization to publish.

## Reported advisory and bounded dependency override

The pilot's Trivy run reported minimatch 10.2.6 in
`integrations/vscode/package-lock.json` affected by CVE-2026-118764 (MEDIUM), with
11.0.1 listed as fixed: https://avd.aquasec.com/nvd/cve-2026-118764 .
Advisory matching alone is not a claim of runtime exploitability.

Release-side dependency inspection found no fixed VSCE parent graph: released
4.1.0 and next 4.1.1-0 still declare minimatch `^10.2.6`. The release owner
authorized an exact parent-scoped override from `@vscode/vsce@4.0.0` to minimatch
11.0.1. It does not override the language client's minimatch 5 dependency.
The inspected VSCE usage calls retained named APIs through CommonJS; removed
APIs are unused. The build's Node 24.19.0 satisfies minimatch 11's Node 22 minimum.
This static inspection alone does not establish upstream support or runtime
compatibility. No advisory suppression or risk acceptance replaces remediation.

On integrated source `9433a4ebb87ddc3b85ce9befbe26eccb176f1c64`, real VSIX
packaging, installed Electron-host behavior and the fresh complete dependency
scan passed. All nine lock inventories were complete and dependency findings
were zero. This resolves the reported build-tool advisory without changing the
language client's minimatch 5 graph.

## Release-side integration and hosted qualification

The release owner accepted reconciliation in the shared sync document.
PR #36 has merged. Integration is in
`/home/jason/wt/daml/build/release-pipeline`, branch
`feat/release-security-integration`, based on main
a9a2e0993483f13c2134cd3a4482a6a82bd5ada0. The pilot branch and the separate
PR #38 lean-build documentation branch remain unchanged.

The cumulative 17-file pilot patch applies without replacing current main's
browser workflow, root LF attributes or canonical delivery specification.
Existing LICENSE.txt admission, generated browser helper probes, explicit tag
materialization and the pinned Playwright CLI remain on the actual producer
and publisher paths. Security runs only in the deliberate preparation collector.
It embeds proof in schema 2; the existing common bundle admission validates it
on publisher preflight, post-approval download, staging and promotion. There
is no independent scanner-report artifact in the producer download set.

The ownership and simplification examinations retain one scanner owner:
`scripts/release_security.py` owns isolated engine/data preparation, snapshot
binding and proof admission. `release_common.py` still owns payload admission,
and `release_publish.py` still owns provider effects. These boundaries avoid a
second publisher or scanner policy in workflow YAML. The strict inventory
parsers and per-check failure handling defend actual supported lock/report
contracts; no integration-specific compatibility layer or refactor was needed.

The independent verification owner admitted the imported checks as
`delivery-2026-11-08-r8`, preserving the existing payload probes and adding
security-proof rejection cases. The probe reseals security asset records with
outer hashes so inner payload validation remains discriminating; schema 3 is
an invalid-schema probe. Main ran 18 delivery and 19 security policy cases,
production Ruff/format, strict OpenSpec validation and the full repository gate
including 151 Python cases successfully.

The pilot's real engines used rootless Docker with a diagnostic ZIP, which was
not shipping qualification. Integrated hosted run
[42530364686](https://github.com/dprb-work/daml/actions/runs/42530364686)
subsequently passed all eight real shipping payloads and the complete configured
suite. Dependencies, secrets, SAST and malware each passed with zero findings;
the collector completed in 1m42s within its 30-minute budget. Native/wheel
consumers on all supported platforms, CPython 3.12/3.13, installed Electron and
packed SDK type/native-parity/Chromium checks passed.

The downloaded schema-2 candidate also passed the independently owned corruption
and staging-boundary probe with source `9433a4e`, run `42530364686` and manifest
SHA-256 `732974da595c81eab7ec7ec840eecbd296cf1a8d2b6c8a6b6cbed4f7e7ecb39c`.
Publication effects were not invoked. Fresh independent reviews found no
actionable source defects. Final-head evidence, including explicit daemon
security options and caller IDs rather than a rootful-mode inference, belongs
in [PR #39](https://github.com/dprb-work/daml/pull/39).

A fresh schema-2 candidate and manifest digest are required; the old schema-1
preparation cannot be reused. GitHub's human environment gate and external PyPI
Trusted Publisher setup remain separate prerequisites, not effects performed
or waived by integration.
