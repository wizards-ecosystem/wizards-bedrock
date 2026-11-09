# D-26 No hosted CI; the local gate is the gate

Status: Accepted, 2026-08-12.

**Context.** A GitHub Actions workflow ran `fmt --check`, clippy, tests, the
vendor digests, and an image build on Linux for every push.

**Decision.** Removed. `cargo xtask ci` is the only gate.

**Rationale.** This is a single-developer project on a single machine, and the
workflow ran exactly the steps `cargo xtask ci` already runs locally in about
twenty seconds — before the push rather than after it. On a private repository
it also consumes a metered minute allowance to tell one person something they
already knew. Branch protection is rejected for the same reason: with no second
author there is no review to enforce, only a barrier to `git push`.

**Consequences.** Nothing now proves the build works on a non-Windows host. That
guarantee was real — it is why `xtask` uses `EXE_SUFFIX` and `QEMU_DIR` instead
of hardcoding `.exe` and a Program Files path — so the portability *code* stays,
untested. Revisit the moment a second person contributes, or if this is ever
built on Linux in earnest; the deleted workflow is recoverable from history.

**Current disposition (2026-08-30).** PW-708 turns that prose into an exact
trigger: restore a supported-host gate when a second active contributor or
serious non-Windows support makes it authoritative rather than duplicative.
