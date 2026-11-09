# D-10 POSIX as a translation layer only; mlibc is the ports gateway

Status: Accepted (amended by D-181)

**Context.** CPython and NetSurf (D-05) require POSIX; the native ABI
(D-06) is not POSIX.

**Decision.** POSIX lives entirely in userspace. Phase 9's mlibc sysdeps make
descriptors compatibility objects over native handles: they add numbering,
shared cursors, inheritance and root/cwd policy while blocking calls wrap ring
submission and wait. A gated compat clone remains Phase 9 work, not a current
kernel primitive. Native software never links this layer (D-181).

**Rationale.** mlibc exists precisely for hobby-OS ports (Managarm
lineage); this preserves both the modern core and the porting trophies.

**Consequences.** Phase 9 is the steepest climb in the roadmap; the
Phase-5 ABI design doc must be written with mlibc's sysdeps checklist
open so no primitive is missing.
