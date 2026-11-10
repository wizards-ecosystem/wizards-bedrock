# D-74 mlibc arrives with the ABI, not after the userland

Status: Accepted, 2026-08-23. Amends D-10.

**Context.** D-10 makes POSIX a userspace translation layer over mlibc, which
the roadmap put in Phase 9, after a native userland in Phase 7. The 2026-08-23
research found that order means writing the userland twice.

It also priced the translation layer, which the plan costed at zero. Fuchsia ran
this experiment (capability handles with POSIX confined to a userspace layer)
and reversed. RFC-0082 records that POSIX Lite omits *"parts of POSIX that imply
mutable global state… because Fuchsia is designed around an object-capability
discipline"*. The answer was Starnix, a Linux kernel in userspace at ≥15 MB of
Rust and 493 syscall-table entries, which implements `fork`. Plan 9's APE hit
the same wall thirty years earlier (uid/gid, umask, controlling tty, `link`,
`sigprocmask`), and Pike estimated the tax: *"90–95% of the work in Plan 9 was
directly or indirectly to honor externally imposed standards."*

mlibc's numbers: 296 sysdep tags, 16 mandatory ("enough for a basic
statically-linked hello world"); real ports implement 147 to 202, and all
implement `Fork`, `Sigaction`, `Ioctl`, `Termios`, `Poll` and `Waitpid`.

**Decision.** A stub-heavy mlibc sysdep is written right after the Phase 5 ABI
stabilises. It runs no real software but answers *"can a POSIX libc sit on
this ABI?"* while the ABI is cheap to change. mlibc ships a five-file starter
skeleton. Pin a release tag, not `master`, per mlibc's own warning.

**Rationale.** The retrospective evidence's strongest correlation is **conform
at the boundary, innovate inside**. Zinnia (one person, started January 2024)
reached a Wayland desktop in about 2.5 years on mlibc, with a sysdep of
thirteen files. Asterinas reached a Linux-compatible userland in about three
years by adopting the Linux ABI outright. Against that: ToaruOS, fifteen years
solo under a strict no-third-party rule, still not self-hosting; HelenOS,
twenty-plus years, elegant, with no path to third-party software.

The capability ABI is not weakened. Redox, eleven years in, is *retrofitting*
capabilities under POSIX, which is strictly harder. The translation layer
should arrive early enough to shape the ABI.

**Consequences.**

- Answer before the queue ABI is fixed: **how does a signal interrupt a
  blocking queue wait?** It is the hardest to answer late, and `002` does not
  yet mention signals.
- Adopting mlibc means adopting `abis/linux`, so the *userspace-visible* ABI
  becomes Linux's. The native ABI is unaffected; `002`'s claim that POSIX is
  "only a translation layer" stays true and gets more specific.
- A POSIX conformance lane (os-test and the Open POSIX Test Suite, counts
  recorded over time) arrives with the mlibc slice as a third check location
  beside `libs/` and `selftest.rs`. Redox and managarm converged on this
  independently; it is D-28 past the kernel boundary.
- CPython's `subprocess` takes its fork path whenever `cwd` is set, among
  other conditions, so a spawn-only OS needs a `posix_spawn` richer than the
  POSIX baseline. That is Phase 9's business, recorded here because D-07 makes
  it true.
