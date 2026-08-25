# The POSIX layer, and what writing it found

D-74 asked for an mlibc sysdep **immediately after the ABI stabilises**, and
was explicit about why: *"not to run real software, but to find out whether a
POSIX libc can sit on this ABI while the ABI is still cheap to change."*

This directory is that answer. It holds `abi/wizard_abi.h` — the native ABI as
a C header — and `sysdeps/wizard/generic.cpp`, mlibc's sixteen mandatory tags
written against it.

**Nothing here is compiled.** The wall is recorded below with its cost, which
is what D-74's fallback asked for rather than a silent omission (D-114).

## The wall

Slice 10's plan was to vendor a portable clang and lld, checksum-pinned under
D-25, and build mlibc against them. Measured rather than estimated:

| | |
|---|---|
| C compiler on this machine | none — no `clang`, `gcc`, `cc`, or `cl` |
| LLVM 19.1.7 Windows release | **845 MB** compressed, ~3 GB extracted |
| ...and it is the **MSVC** build | needs MSVC headers and libs, so not self-contained (D-16) |
| mlibc's build system | meson + ninja, neither present, both Python-ecosystem installs |
| mlibc target triple | none exists for this OS; adding one patches its meson files |

The self-contained rule (D-16) is what decides it. LLVM ships no mingw-based
Windows release, so the vendored clang would depend on an MSVC installation
this machine does not have and the project has no way to pin — which is the
opposite of the property vendoring exists to buy. The download is the smaller
objection.

## What was learned anyway, which is the point

The tags divide three ways. The counts are the finding.

| | Count | |
|---|---|---|
| **MAPS** | 4 | the native ABI already has the shape POSIX wants |
| **OWED** | 11 | the kernel has the mechanism and no ring-3 door |
| **ABSENT** | 1 | the native ABI has nothing of the kind, deliberately |

**MAPS (4):** `sys_libc_panic`, `sys_exit`, `sys_tcb_set`, and the *shape* of
`sys_libc_log`. Two work end to end today: `sys_exit` retires a ring-3 thread
from the far side of the boundary (D-96), and `sys_tcb_set` points `fs:` at
thread-local storage that then follows the thread across a switch (D-118).

**OWED (11):** every one has a kernel-side mechanism and no door. `sys_close`
over the handle table's close (D-78). `sys_read`/`sys_write` over the ring
(D-113). `sys_vm_map`/`sys_vm_unmap` over `map_user` and `unmap` (D-107,
D-88). `sys_futex_wait`/`sys_futex_wake` over `WaitQueue::wait_unless`, which
already solves the lost-wakeup window a futex gets wrong first (D-95).
`sys_clock_get` over `libs/timebase`.

**ABSENT (1):** `sys_open` needs a *namespace*, and there is no global
filesystem to name a path in — by design (pillar 1). A libc can be built where
`open` resolves through a directory handle the process was given, which is
`openat` with no `AT_FDCWD` and is what WASI's `path_open` settled on for the
same reason. What such a libc cannot do is `open("/etc/foo")` from a process
nobody gave a root directory to, and that is the point rather than a defect.

### Four things the sysdeps surfaced that the native ABI would never have raised

The first is fixed, and it is the clearest return the exercise paid: it was a
real hole, in a place three neighbouring decisions had already visited, and
only a POSIX consumer would have looked there.

1. ~~**`FS_BASE` is a fourth thing that must follow a thread.**~~ **Fixed by
   D-118**, and it is the clearest return this exercise paid: `RSP0` (D-99),
   `CR3` (D-106) and the extended state (D-108) all lived in `task::prepare`
   and this did not, because nothing native has thread-local storage and so
   nothing native had asked. `sys_tcb_set` maps now, and the kernel range-checks
   the base — a `wrmsr` of a non-canonical value is a fault in kernel context,
   which would be a program crashing the handler rather than itself.

2. **There is no wall clock.** `sys_clock_get` wants `CLOCK_REALTIME` and this
   kernel has never been told what time it is. Uptime is monotonic and is all
   there is. A libc can run on that; `stat` timestamps and `time()` cannot.

3. **A seek is state the *object* holds, not the handle.** Two POSIX
   descriptors to one file share a position. That is a distinction the kernel
   will have to make about a file object, which by D-78's rule makes it a
   legitimate per-type right — and it is a distinction nothing native needed.

4. **W^X is a real incompatibility and the right one to have.**
   `PROT_WRITE | PROT_EXEC` is refused rather than granted (D-12, D-100), so a
   JIT must map twice. Naming it here means a port that fails on it fails for a
   stated reason.

### The two questions D-74 said were cheaper to answer now

**How a signal interrupts a blocking queue wait.** A signal is a completion
entry (D-80). There is no second event plane and no in-kernel signal frame:
`sigaction` runs entirely in userspace off a completion, dispatched by the
libc's own ring loop. The consequence, which writing the sysdeps made concrete:
**every blocking libc entry point must be a ring submission plus a wait**, not
a synchronous call — because a thread blocked in a synchronous call cannot be
interrupted at all. That is why `sys_read` above is written as a submission and
not as a syscall, and it is a constraint on the whole libc rather than on
signals.

**Static-only, or a dynamic linker.** Static-only for now. CPython needs
`dlopen` and `-export-dynamic`, which is Phase 9's problem; the answer is
cheaper now because a dynamic linker needs `mmap` with `PROT_EXEC` on a file
mapping, and the file-backed half of `sys_vm_map` is on the OWED list rather
than the ABSENT one. Nothing in the ABI forecloses it.

### The cluster that stays unimplementable

uid/gid, umask, controlling tty, hard links, and signal masks. This is the same
cluster Plan 9's APE could not implement and Fuchsia excluded from POSIX Lite,
and it is unimplementable here for the same reason: each names an *ambient*
authority, and a process's power is exactly its handle table. None of them is
among the sixteen mandatory tags, which is itself informative — mlibc's own
minimum does not require them.

## What is owed

A compiled hello-world, which needs a self-contained C toolchain this platform
does not currently offer. The realistic routes, in order of cost:

1. Build LLVM from source against the mingw toolchain already present. Hours,
   and a large vendored artefact afterwards.
2. Cross-compile from a machine that has a C toolchain, and vendor the
   *result*. Breaks the "one machine, self-contained" property D-16 is about.
3. Wait for a mingw-based LLVM Windows release, and check periodically.

The sysdeps and this analysis are the part that answers D-74's question. The
compile is the part that confirms it, and it is deferred with its cost named.
