## 1. Contract and checks

- [x] Inspect existing module admission, Python package and verification owners.
- [x] Record the user-approved Python/Pydantic/dataclass/custom-exporter boundary and supported projection.
- [x] Freeze independently authored Python regression cases for positive, rejection and optional-dependency boundaries.

## 2. Implementation

- [x] Add the Python helper and optional Pydantic dependency without native implementation changes.
- [x] Preserve existing module scopes/errors and reject unrepresentable behavior without fallback or coercion.
- [x] Add an executable standalone model/dataclass example, a separate custom-exporter demonstration and maintained documentation.

## 3. Verification and handoff

- [x] Run the frozen regression suite and actual model/dataclass/msgspec consumer scenarios.
- [x] Prove base import/custom export without the optional dependency; check an extracted built package.
- [x] Run repository-native checks and OpenSpec validation on the exact candidate.
- [ ] Obtain independent code/security/contract review, repair kept findings and re-review.
- [ ] Publish verified source for owner handoff; report remote CI and any pending integration authority separately.
