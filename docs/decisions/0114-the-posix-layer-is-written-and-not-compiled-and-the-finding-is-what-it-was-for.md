# D-114 The POSIX layer is written and not compiled, and the finding is what it was for

Status: Accepted (2026-08-24) (amended by D-181), amends D-74

**Context.** D-74 asked for an mlibc sysdep "immediately after the ABI
stabilises, not to run real software, but to find out whether a POSIX libc can
sit on this ABI while the ABI is still cheap to change". Slice 10's plan was to
vendor a portable clang and lld under D-25 and build against them, and it named
its own fallback: if the vendored clang cannot build mlibc after a bounded
attempt, author the sysdeps and the ABI surface, **record the wall with its
cost**, and close without a compiled libc — an ADR amending D-74, not a silent
omission.

**The wall, measured rather than estimated on 2026-08-24.** At that measurement
this machine had no C compiler. LLVM 19.1.7's Windows release was 845 MB
compressed and about 3 GB extracted, and — the objection that actually decided
it — it was the **MSVC** build. LLVM shipped no mingw-based Windows release, so
a vendored clang would have depended on an unpinned MSVC installation. mlibc
additionally needed meson and ninja, neither then present, and a target triple
that did not exist. D-181 records the later tool recheck; newly appeared local
executables still do not constitute a pinned source/toolchain/target closure.

**Decision.** Take the fallback, and make it a deliverable rather than a
shrug. At this decision, `fixtures/abi-pressure/abi/wizard_abi.h` transcribed the native ABI
for C and `fixtures/abi-pressure/sysdeps/wizard/generic.cpp` wrote the sixteen functions
believed mandatory at an unrecorded mlibc snapshot, each marked with one of
three labels. The counts were the finding: **3 MAPS, 12 OWED, 1 ABSENT** — then
revised to **4 MAPS, 11 OWED, 1 ABSENT** by D-118, which paid `sys_tcb_set`.
D-124 checked those counts against the sysdeps' labels. They remain here as the
dated result; D-181 retires the one-axis premise and replaces the current view
with an exact two-axis register.

The contemporaneous claim was that the OWED all had a kernel-side mechanism
and no ring-3 door: `sys_close` over the handle table's close, reads and writes
over the ring, memory calls over `map_user` and `unmap`, and futexes over
`WaitQueue::wait_unless`. D-181 found the premise false: write was absent,
read and seek need compatibility cursor state, and an internal wait queue is
not a userspace-address-keyed futex.

The one ABSENT was `sys_open`, for the intended lack of an ambient namespace.
Phase 6 subsequently built read-only OpenAt. D-181 records useful substrate but
not a complete route: no ring-3 bootstrap/channel operation can deliver the
first directory yet, and the POSIX adapter must still supply root/cwd/descriptor
policy while refusing mutating modes. A process nobody gave a root directory
still cannot open `/etc/foo`, which remains the point.

**Four things the exercise surfaced that the native ABI would never have
raised**, which is precisely the value D-74 predicted:

1. **`FS_BASE` is a fourth thing that must follow a thread.** At this decision,
   `sys_tcb_set` exposed that the kernel did not save it across a switch.
   D-118 closed the finding: it now lives in `task::prepare` beside `RSP0`
   (D-99), `CR3` (D-106) and the extended state (D-108).
2. **There is no wall clock.** Uptime is monotonic and is all there is; this
   kernel has never been told what time it is. `time()` and `stat` timestamps
   need an epoch.
3. **The contemporaneous finding put seek state on the file object.** D-181
   corrects that ownership: the native protocol is deliberately cursorless;
   the POSIX adapter owns an open-file-description cursor shared by duplicated
   descriptors. It is compatibility state, not a native right.
4. **W^X is a real incompatibility and the right one.** `PROT_WRITE|PROT_EXEC`
   is refused (D-12, D-100), so a JIT maps twice. Named so a port that fails on
   it fails for a stated reason.

**The two questions D-74 said were cheaper now, narrowed by D-181.** D-80
requires signals to share the completion plane, but version 1 has no
unsolicited-event discriminator; the old colliding signal opcode did not make
`sigaction` exist. A future representation still requires blocking libc work
to use an interruptible ring wait. Userland is static-only for now. A dynamic
linker remains possible, but user mapping and file-backed execution are
PARTIAL platform work rather than a pre-numbered owed call.

**Consequences, amended by D-124 and D-181.** The header is a second
transcription for a C consumer, not the ABI's authority. The current host check
removes comments from both languages, inventories every header definition,
compares the complete call/opcode domains and active wire definitions with
kernel source, and derives the entry and time structures' offsets, widths and
sizes. The sysdep check independently discovers all active definitions and
checks each BODY claim against its body.
The original small comparison was broken in both directions; D-181's stronger
checks were broken in the additional ways recorded there.

**What is owed, with its route.** A compiled hello-world. Phase 9 must pin the
mlibc revision, the compiler/Python/Meson/Ninja closure and the Wizard target,
then build the transcription it actually adopts. The sysdeps and analysis are
the early pressure test D-74 asked for; only that reproducible build confirms a
port.
