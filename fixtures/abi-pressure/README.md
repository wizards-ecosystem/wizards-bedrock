# The POSIX layer, and what writing it found

This is a Bedrock kernel ABI fixture retained under D-366. It is uncompiled
historical contract evidence. Executable compatibility adapters and ports are
owned by Realm; no Realm checkout is read by these checks.

D-74 asked for an mlibc sysdep **immediately after the ABI stabilises**, and
was explicit about why: *"not to run real software, but to find out whether a
POSIX libc can sit on this ABI while the ABI is still cheap to change."*

This directory is that pressure test. It holds `abi/wizard_abi.h`, a C
transcription of the native ABI, and `sysdeps/wizard/generic.cpp`, the
sixteen functions inspected for D-114.

**Nothing here is compiled, and no mlibc source revision was recorded.** These
files are neither a current upstream port nor build evidence. They preserve
the consumer questions D-74 wanted answered; Phase 9 must pin the API it
actually builds before treating a signature here as authoritative (D-181).

## The wall

Slice 10 planned to vendor a portable clang and lld under D-25 and build mlibc
against them. D-114 recorded this dated measurement:

| | |
|---|---|
| C compiler on this machine | none: no `clang`, `gcc`, `cc`, or `cl` at that measurement |
| LLVM 19.1.7 Windows release | **845 MB** compressed, ~3 GB extracted |
| ...and it is the **MSVC** build | needs MSVC headers and libs, so not self-contained (D-16) |
| mlibc's build system | meson + ninja, neither present at that measurement |
| mlibc target triple | none exists for this OS; adding one patches its meson files |

That environment changed. A 2026-08-26 recheck found unpinned WinLibs
GCC/G++ 16.1, Ninja 1.13.2, and Python 3.14.3 through the working `py`
launcher (`python` and `python3` are non-working Store aliases). Meson, pinned
mlibc source, Wizard target integration, and a repository-owned toolchain are
still absent. Tool appearance does not turn the historical fallback into a
reproducible build. The wall now is the unpinned
source/toolchain/build-system/target closure, not the false timeless claim that
this workstation has no compiler or Python runtime.

## What was learned anyway, which is the point

D-181 retired the three-bin tally because it mixed two different questions
and was false in both directions: a body could be a no-op while its *shape*
was called mapped, and an internal wait queue was called an owed futex even
though the userspace-keyed mechanism did not exist.

D-182 then tightened the platform axis: an internal operation is not merely a
missing door when calling it can allocate, deallocate or reclaim from forbidden
trap context. That distinction moves close, unmap and anonymous free to
PARTIAL until they have a safe execution route.

The **BODY** column describes only the uncompiled C++:

- **WIRED** invokes a call the kernel currently decodes.
- **COMPOSED** obtains its required behavior from another entry.
- **PARTIAL** has a real success path for a strict subset and refuses the rest.
- **STUB** has no success path.

The **REMAINDER** column describes platform work, not whether this C++ has ever
run:

- **WORKS** means no known platform semantic is missing for that function.
- **COMPAT** needs POSIX-only state or policy over existing native operations.
- **DOOR** has a matching internal kernel operation but no chosen user entry.
- **PARTIAL** has useful substrate or a semantic subset, but not the contract.
- **MISSING** lacks the required kernel operation rather than merely an adapter.

| Sysdep | BODY | REMAINDER | Exact reason |
|---|---|---|---|
| `sys_libc_log` | STUB | PARTIAL | It is a silent no-op; console and terminal pieces have no write operation joining them |
| `sys_libc_panic` | COMPOSED | PARTIAL | It terminates through thread-only `sys_exit`; process-wide death and observation remain absent, while diagnostic output is not required to stop |
| `sys_exit` | WIRED | PARTIAL | Call 0 retires only the current thread and discards its status; process lifecycle and an observer do not exist |
| `sys_clock_get` | PARTIAL | PARTIAL | The checked page supplies monotonic time; realtime and the correlated Uptime fallback do not exist |
| `sys_read` | STUB | COMPAT | Native Read takes an explicit offset; POSIX read needs shared cursor state |
| `sys_write` | STUB | MISSING | There is no VFS or ring write path |
| `sys_seek` | STUB | COMPAT | The native protocol is deliberately cursorless |
| `sys_close` | STUB | MISSING | No close operation exists before the Wizzy caller defines its trap-safe lifetime contract |
| `sys_open` | STUB | PARTIAL | Read-only OpenAt exists, but no ring-3 bootstrap/channel path can deliver the first directory; compatibility must also supply root/cwd/descriptors and reject mutating modes |
| `sys_vm_map` | STUB | PARTIAL | Loader primitives exist; allocation and the non-blocking trap/deferred execution contract do not |
| `sys_vm_unmap` | STUB | PARTIAL | Address-space unmap exists, but a user operation must couple its flush and reclamation obligations to a trap-safe execution route |
| `sys_anon_allocate` | STUB | PARTIAL | It depends on the unresolved mapping/allocation contract |
| `sys_anon_free` | STUB | PARTIAL | It inherits unmap's unresolved flush, reclamation and trap-execution contract |
| `sys_futex_wait` | STUB | MISSING | An internal wait queue is not a futex keyed by validated userspace memory |
| `sys_futex_wake` | STUB | MISSING | No userspace-address-keyed wake operation exists |
| `sys_tcb_set` | WIRED | WORKS | Call 2 validates, installs, saves and restores `FS_BASE` |

The host check discovers the sixteen active definitions, checks each BODY's
evidence in comment-free code, and requires the README row and source marker to agree.
REMAINDER is reviewed platform judgment; targeted kernel-source witnesses guard
the disputed Read/Close/Write/futex/panic/exit distinctions. Aggregate counts are
deliberately not a second source of truth.

### Four things the sysdeps surfaced that the native ABI would never have raised

The first is fixed, and it is the clearest return the exercise paid: it was a
real hole, in a place three neighbouring decisions had already visited, and
only a POSIX consumer would have looked there.

1. ~~**`FS_BASE` is a fourth thing that must follow a thread.**~~ **Fixed by
   D-118**, and it is the clearest return this exercise paid: `RSP0` (D-99),
   `CR3` (D-106) and the extended state (D-108) all lived in `task::prepare`
   and this did not, because nothing native has thread-local storage and so
   nothing native had asked. `sys_tcb_set` is now WIRED with no remaining
   platform gap, and the kernel range-checks the base: a `wrmsr` of a
   non-canonical value is a fault in kernel context, which would be a program
   crashing the handler rather than itself.

2. **There is no wall clock.** `sys_clock_get` wants `CLOCK_REALTIME` and this
   kernel has never been told what time it is. Uptime is monotonic and is all
   there is. A libc can run on that; `stat` timestamps and `time()` cannot.

3. **A seek and read share an open-file-description cursor.** Native Read
   deliberately takes an offset per operation. The POSIX adapter must therefore
   construct and share cursor state across duplicated descriptors; calling
   Read implemented did not make either sysdep a thin adapter.

4. **W^X is a real incompatibility and the right one to have.**
   `PROT_WRITE | PROT_EXEC` is refused rather than granted (D-12, D-100), so a
   JIT must map twice. Naming it here means a port that fails on it fails for a
   stated reason.

### The two questions D-74 said were cheaper to answer now

**How a signal interrupts a blocking queue wait.** D-80 requires signals to
share the completion plane and rejects in-kernel signal frames. The v1 record
cannot express that yet: a completion has no kind or opcode, and every token
belongs to a submitted operation. The old C header assigned
`WIZ_OP_SIGNAL = 3`, colliding with Duplicate while placing an opcode in a
record that has none. D-181 deletes that fiction. Phase 9 must version a real
unsolicited-event representation and keep every blocking libc entry point on
an interruptible ring wait; the one-plane constraint survives, but no wire
format is claimed before its caller.

**Static-only, or a dynamic linker.** This historical pressure test assumes
static linkage. CPython needs `dlopen` and `-export-dynamic`, which is Phase
9's problem; a dynamic linker also needs `mmap` with `PROT_EXEC` on a file
mapping. The loader has mapping substrate, but no user mapping/allocation
contract or file-backed door; the register says PARTIAL. Nothing in the ABI
forecloses a later static-to-dynamic transition.

### The cluster that stays outside the native ABI

uid/gid, umask, controlling tty, hard links, and signal masks are unsupported
POSIX mutable-global or ambient conventions. They need explicit compatibility
state, capability routing or policy rather than native defaults. Plan 9's APE
and Fuchsia POSIX Lite are evidence of that cost, not an impossibility proof:
`linkat` can be rooted in directory handles, a TTY can be delegated, and masks
and umask can be adapter state. None is among the sixteen functions D-114
inspected, which bounds what this pressure test says about them.

Two more belong here and were named in the sysdeps without being recorded
anywhere, which is how a forward reference becomes a dead one:

- **`dup2` to a chosen number.** Duplicating a handle is fine; choosing *which*
  value the copy gets is not. D-70 rule 7 gives the table no lowest-free
  promise precisely so that two allocations in either order produce the same
  two handles, which is what makes allocation commutative and the table
  scalable. A caller that may name the result has re-introduced the ordering
  the rule deletes. Native duplication exists; chosen-number duplication does
  not.
- **Inheritance across `exec`.** A spawned process receives exactly one opaque
  bootstrap identity and inherits no table. D-248 removed the speculative
  channel transport; the future Wizzy bootstrap caller must define how grants
  arrive. Later local authority can still only be derived through an
  already-held handle (D-111, D-151, D-163). Inheriting a table by default is
  the opposite rule, and it is the one that makes POSIX's `CLOEXEC` necessary:
  a flag that exists to undo a default nobody wanted. There is nothing to undo
  here.

## What remains before this is a port

Phase 9 must pin an mlibc revision, derive the exact sysdep surface that revision
actually requires, and reconcile it with this historical sixteen-function
register. It must also pin or vendor the compiler plus Python/Meson/Ninja
closure, add the Wizard target and linker integration, implement the resulting
register, and compile a static hello-world. Only that build can replace
source-shape checks with runtime evidence. D-114's uncompiled exercise answered
the early design question; it did not pre-authorize ABI numbers or prove a
libc.
