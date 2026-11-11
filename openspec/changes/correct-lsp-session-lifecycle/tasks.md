## 1. Accept the integrated scope and freeze evidence

- [x] 1.1 Record the accountable owner's decision on the integrated design, pinned dependency corrections and 30-second startup deadline; implementation remains blocked until that decision exists.
- [x] 1.2 Freeze behavioral checks with an independent check owner: paused outgoing sends, RPC settlement under delayed/failing writes, missing context/initialization responses, crash policy, failed retirement, protocol deadlines and installed-host restart. Retain the existing race and rejection-probe evidence without replaying the old CI merely to confirm its failure.

## 2. Correct dependency resource and promise ownership

- [x] 2.1 Add one explicit preparation owner for exact-version dependency corrections after script-disabled installation; prove clean installation, repeat preparation and drift refusal without implicit postinstall or network fallback, preserving package provenance and notices.
- [x] 2.2 Correct pinned JSON-RPC tracking, rejection and cancellation-listener settlement; prove disposal during a pending write and deliberate retirement reject each caller once without unhandled rejection or synthetic success.
- [x] 2.3 Correct captured-connection disposal on failed and timed-out stop; prove outstanding callers settle and the original error remains visible.
- [x] 2.4 Extend existing Base/Node start-stop ownership to cancel a captured Starting attempt; prove cancellation before acquisition, immediately before spawn and during initialize, with no pre-initialization shutdown, leaked late child or stale Running publication.

## 3. Establish one client lifecycle and wire-admission owner

- [x] 3.1 Replace implicit absent-client control with an owned transition/session record and desired-configuration reconciliation; prove obsolete startup, A-to-B-to-C changes, failed retirement and deactivation cannot publish or reactivate stale sessions.
- [x] 3.2 Add the connection-scoped final writer gate through the supported transport override while retaining the stock launcher; prove FIFO for accepted writes and absence of every post-cutover application-message class, including resumed providers, full-sync flushes, cancellation, trace, progress, watchers and delayed replies.
- [x] 3.3 Remove business-response lifetimes from lifecycle serialization while retaining session-local context ordering, document identity and generation rules; withhold a context response and prove retirement still progresses and stale results install no watchers.
- [x] 3.4 Route delegated stock crash-restart decisions through reconciliation with the same retained policy history; prove intentional retirement, stale callbacks, untrusted workspaces and terminal disposal cannot trigger or reset automatic replacement.
- [x] 3.5 Apply the startup deadline to the exact attempt outside its blocked startup wait; prove expiry and earlier deactivation retire it, preserving error/cancellation identity and bounded process cleanup.

## 4. Establish protocol terminal admission

- [x] 4.1 Replace the server shutdown helper with a terminal phase under `serve`; prove one null acknowledgment, correlated InvalidRequest for every later request, no post-shutdown session effects, and clean exit after violations.
- [x] 4.2 Use one absolute deadline for terminal channel sends/receives with explicit expiry checks; prove sustained traffic cannot extend it and failures do not hang on open stdin or become successful shutdown.
- [x] 4.3 Preserve initialization correction, request-before-shutdown ordering, premature-exit failure, payload-safe errors and successful stdio joining with stdin held open through the existing protocol checks.

## 5. Verify the assembled product and hand off

- [x] 5.1 Package the corrected VSIX through normal preparation/build paths and use the matching source-built native server; independently verify client wire ordering and server behavior rather than using a tolerant server as proof of prevention.
- [x] 5.2 Run real installed-host invalid-format/restart/replacement-diagnostics and non-exec-wrapper cleanup scenarios, plus existing semantic/watcher/trust checks; report the platform scope actually exercised.
- [x] 5.3 Update affected lifecycle/build/verification documentation and dependency provenance; run focused checks, strict OpenSpec validation and the applicable repository-native aggregates on the reviewed implementation.
- [x] 5.4 Obtain exact-head independent implementation review.
- [x] 5.5 Obtain owner result acceptance. Daniel accepted the verified result and authorized pushing this branch, opening a PR to `main`, and squash-merging after required checks pass in the 2026-11-11 integration handoff. No new release, tag or publication is authorized; v0.2.1 remains unchanged.

Verified implementation: `e029d19d0631e4df27bfaa7767144abbbcfb81d3`.
`./scripts/check`, editor typecheck, strict OpenSpec validation and the complete
installed-editor aggregate pass locally. The latter includes real SDK/provider
consumers, both root-controller cases, all six adversarial scenarios and semantic
authoring on Linux with VS Code 1.105.1. Independent lifecycle, contract and
security reviews found no remaining source defects. Remote CI and Windows/macOS
runtime behavior are not claimed. The documented pinned-host output-channel
factory limitation remains separate from the repaired SDK resource ownership.
