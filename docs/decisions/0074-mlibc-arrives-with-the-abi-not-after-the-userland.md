# D-74 mlibc arrives with the ABI, not after the userland

Status: Accepted, 2026-08-23. Amends D-10.

**Decision.** A stub-heavy mlibc sysdep is written right after the Phase 5
ABI stabilises. It runs no real software but answers *"can a POSIX libc sit
on this ABI?"* while the ABI is cheap to change. Pin a mlibc release tag,
not `master`, per mlibc's own warning.

**Rationale.** Waiting until Phase 9, after a native userland, means writing
the userland twice, and the translation layer is not free: Fuchsia confined
POSIX to userspace and ended up with Starnix, and every real mlibc port
implements `Fork`, `Sigaction`, `Ioctl`, `Termios`, `Poll` and `Waitpid`.
mlibc has 296 sysdep tags, 16 mandatory ("enough for a basic
statically-linked hello world"); real ports implement 147 to 202. The
strongest pattern in the retrospective evidence is **conform at the
boundary, innovate inside**. The capability ABI is not weakened; the
translation layer should arrive early enough to shape it.

**Consequences.**

- Answer before the queue ABI is fixed: **how does a signal interrupt a
  blocking queue wait?** It is the hardest to answer late, and `002` does
  not yet mention signals.
- Adopting mlibc means adopting `abis/linux`, so the *userspace-visible* ABI
  becomes Linux's. The native ABI is unaffected; `002`'s claim that POSIX is
  "only a translation layer" stays true and gets more specific.
- A POSIX conformance lane (os-test and the Open POSIX Test Suite, counts
  recorded over time) arrives with the mlibc slice as a third check location
  beside `libs/` and `selftest.rs`: D-28 past the kernel boundary.
- CPython's `subprocess` takes its fork path whenever `cwd` is set, among
  other conditions, so a spawn-only OS needs a `posix_spawn` richer than the
  POSIX baseline. That is Phase 9's business, recorded here because D-07
  makes it true.
