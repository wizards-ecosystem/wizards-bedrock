# D-26 No hosted CI; the local gate is the gate

Status: Accepted, 2026-08-12.

**Decision.** The GitHub Actions workflow is removed. `cargo xtask ci` is
the only gate. Branch protection is rejected.

**Rationale.** On a single-developer project the workflow repeated the local
gate after the push instead of before it, and with no second author branch
protection has no review to enforce.

**Consequences.** Nothing now proves the build works on a non-Windows host,
so the portability *code* (`EXE_SUFFIX`, `QEMU_DIR`) stays, untested.
Revisit when a second person contributes, or if this is ever built on Linux
in earnest.

**Current disposition (2026-08-30).** PW-708 turns that prose into an exact
trigger: restore a supported-host gate when a second active contributor or
serious non-Windows support makes it authoritative, not duplicative.
