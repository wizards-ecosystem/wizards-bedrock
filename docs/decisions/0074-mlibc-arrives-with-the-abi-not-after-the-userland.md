# D-74 mlibc arrives with the ABI, not after the userland

Status: Accepted (2026-08-23), amends D-10

**Context.** D-10 makes POSIX a userspace translation layer over mlibc, and the
roadmap put it in Phase 9 — after a native userland in Phase 7. The
2026-08-23 research priced that ordering and found it means writing the
userland twice.

It also priced the translation layer itself, which the plan had costed at zero.
Fuchsia ran this exact experiment — capability handles with POSIX confined to a
userspace layer — and reversed: RFC-0082 records that POSIX Lite omits *"parts
of POSIX that imply mutable global state… because Fuchsia is designed around an
object-capability discipline"*, and the answer was Starnix, a Linux kernel in
userspace at ≥15 MB of Rust and 493 syscall-table entries, which implements
`fork`. Plan 9's APE hit the identical wall thirty years earlier — uid/gid,
umask, controlling tty, `link`, `sigprocmask` — and Pike's estimate of the tax
was that *"90–95% of the work in Plan 9 was directly or indirectly to honor
externally imposed standards."*

mlibc's own numbers: 296 sysdep tags, 16 mandatory ("enough for a basic
statically-linked hello world"), and real ports implement 147 to 202. Every one
implements `Fork`, `Sigaction`, `Ioctl`, `Termios`, `Poll` and `Waitpid`.

**Decision.** A stub-heavy mlibc sysdep is written immediately after the Phase 5
ABI stabilises — not to run real software, but to answer *"can a POSIX libc sit
on this ABI?"* while the ABI is still cheap to change. mlibc ships a five-file
skeleton to start from. Pin a release tag, not `master`, per mlibc's own
warning.

**Rationale.** The strongest single correlation in the retrospective evidence is
**conform at the boundary, innovate inside**. Zinnia — one person, started
January 2024 — reached a Wayland desktop in about 2.5 years on mlibc, with a
sysdep of thirteen files. Asterinas reached a Linux-compatible userland in about
three years by adopting the Linux ABI outright. Against that: ToaruOS, fifteen
years solo under a strict no-third-party rule, still not self-hosting; HelenOS,
twenty-plus years, elegant, with no path to third-party software.

This does not weaken the capability ABI. Redox is eleven years in and
*retrofitting* capabilities underneath POSIX, which is strictly harder. It says
the translation layer should arrive early enough to shape the ABI rather than
inherit it.

**Consequences.**

- The question to answer before the queue ABI is fixed: **how does a signal
  interrupt a blocking queue wait?** It is the one that is hardest to answer
  late, and `002` currently does not mention signals at all.
- Adopting mlibc means adopting `abis/linux`, so the *userspace-visible* ABI
  becomes Linux's. The native ABI is unaffected; the claim in `002` that POSIX
  is "only a translation layer" stays true and gets more specific.
- A POSIX conformance lane — os-test and the Open POSIX Test Suite, counts
  recorded over time — arrives with the mlibc slice as a third check location
  beside `libs/` and `selftest.rs`. Both Redox and managarm converged on this
  independently, and it is D-28 extended past the kernel boundary.
- `subprocess` in CPython takes its fork path whenever `cwd` is set, among
  other conditions, so a spawn-only OS needs a `posix_spawn` richer than the
  POSIX baseline. Phase 9's business, recorded here because D-07 is what makes
  it true.
