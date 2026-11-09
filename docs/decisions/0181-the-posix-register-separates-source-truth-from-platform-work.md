# D-181 The POSIX register separates source truth from platform work

Status: Accepted (2026-08-26), amends D-06, D-10, D-72, D-80, D-114, D-116, D-118, D-149, D-150, D-151, D-152, D-164, D-166, D-169 and D-173

**Context.** The D-180 audit followed Phase 5's mlibc pressure test back to its
evidence and found that the checked 5/10/1 tally was precise arithmetic over
false categories. `sys_libc_log` was counted as MAPS while its body was a
silent no-op. `sys_read` had a shipped explicit-offset ring operation, while
`sys_write` had no operation at all; calling both OWED hid the difference.
`sys_seek` needs POSIX open-file-description cursor state the native protocol
deliberately omits. `WaitQueue::wait_unless` closes an internal wakeup window,
but is not a futex keyed by a validated userspace atomic and its lifetime.
Phase 6 built native OpenAt, so calling the entire `sys_open` mechanism ABSENT
confused the rejection of an ambient root with the absence of relative lookup.

The implementation transcription disagreed with its labels too.
`sys_clock_get` ignored the requested clock ID and returned monotonic uptime
as realtime, sampled a bare `rdtsc` after the sequence snapshot, and performed
the fixed-point multiply in `u64`. At a 3 GHz counter that multiply can wrap
in a few seconds. `sys_close` invoked proposed call 4 even though the kernel
decodes only 0–3. The header numbered eight such unimplemented calls, assigned
`WIZ_OP_SIGNAL = 3` on top of Duplicate, and described signals as opcodes even
though a completion has no opcode field or unsolicited-event discriminator.

D-114 also recorded no mlibc source revision. Its C++ has never been compiled.
The local toolchain measurement changed by 2026-08-26—WinLibs GCC/G++ 16.1,
Ninja 1.13.2 and Python 3.14.3 through `py` appeared (`python` and `python3`
remain non-working Store aliases). Meson, pinned mlibc source, Wizard target
integration and a repository-owned toolchain are still absent. A
current-looking function signature is therefore not current upstream evidence,
and executables found on one workstation are not a D-16 build.

**Decision.** Preserve D-114's counts as a dated pressure-test result and
retire its one-dimensional taxonomy for present-tense claims. Every one of the
sixteen inspected functions now has two separate fields in
`fixtures/abi-pressure/README.md`.

The **BODY** axis describes only active code in the uncompiled transcription:
WIRED invokes a call the kernel decodes; COMPOSED obtains its required
behavior from another entry; PARTIAL has a valid success path for a strict
subset; STUB has none. The **REMAINDER** axis describes platform work: WORKS
has no known missing platform semantic; COMPAT needs POSIX-only state or
policy over existing native operations; DOOR has a matching internal
operation but no user entry; PARTIAL has useful substrate or a semantic
subset; MISSING lacks the kernel operation itself.

The exact per-name register is canonical rather than another prose count.
Notable consequences of applying it are that Read and Seek share COMPAT
because both need the same cursor; Exit is WIRED/PARTIAL because the kernel
retires only the calling thread and discards its status; Panic is
COMPOSED/PARTIAL because composing that Exit cannot supply process-wide death
or observation; and the futex entries are STUB/MISSING rather than promises
that WaitQueue is nearly a futex. Open is STUB/PARTIAL: OpenAt exists, but the
first directory still has no ring-3 delivery path in addition to its adapter
policy.

**Repair the consumer, not only its labels.** The time reader now accepts only
`CLOCK_MONOTONIC`, validates the constants version, page extent and counter
rate, reads a doubly fenced TSC inside the seqlock window, validates the
conversion, uses a 128-bit intermediate and saturates rather than wraps. Page
revocation, an incompatible layout and an unstable snapshot route to one
explicit ENOSYS fallback. That fallback remains incomplete until a shared
ring client can submit Uptime with a unique token and reap its matching
out-of-order completion without stealing another caller's; the register
therefore stays PARTIAL/PARTIAL. Realtime remains absent until the system has
an epoch.

The surrounding page contract also claimed calibration required CPUID's
invariant-TSC bit. D-57's live policy does the opposite deliberately: two
believable, agreeing windows admit the clock and a missing invariant promise
produces a warning. The page and kernel clock share that fixed conversion, so
they agree even when the hardware guarantee is absent; neither becomes more
stable than the underlying counter. D-225 later supplied automatic post-boot
revocation; suspend/resume and NTP adjustment remain later policy.

The C header now assigns numbers only to the four calls `Call::decode`
accepts. Proposed close, duplicate, channel, mapping and futex call numbers are
deleted; Duplicate already belongs to the ring, channels are planned as ring
traffic, mapping's execution contract is unresolved, and the futex substrate
does not exist. The colliding signal define is deleted. D-80's one-event-plane
constraint remains, but a signal representation waits for a versioned record
and its first caller. Unused errno defines are removed, the Duplicate comment
uses the actual separate handle and rights fields, and the header distinguishes
handle-authorized objects from its enumerated ambient ring/time facilities.
`WRITE`, `MODIFY`, `EXECUTE` and `MAP` remain transcribed raw rights, with the
explicit fact that no current File admits `WRITE`, no current Directory admits
`MODIFY`, and no current object type admits `EXECUTE` or `MAP`.

The same audit found two authority defects behind the prose. OpenAt gave every
file `UNIVERSAL | READ`, recreating universal rights even when the directory
was held with `TRAVERSE` alone; it now intersects the parent's universal rights
before deriving file `READ`, with a ring check beginning from no universal
rights. The File and Directory object masks also admitted `WRITE` and `MODIFY`
although neither operation exists. Those masks are now `READ` and
`TRAVERSE | ENUMERATE`, and the meaningless-right refusal check covers File
`WRITE`/`EXECUTE`/`MAP` plus Directory `MODIFY`. Persistent mutations restore
each defect.
OpenAt also copied the submitted path before reserving its result slot, while
Duplicate's slot-first comment hid rights validation ahead of its reservation.
Both contradicted D-151's common admission order. Reservation now comes first;
the full-table check pairs exhausted quota with a valid Duplicate, malformed
Duplicate rights and a malformed OpenAt path, so either validation moving ahead
of the promise changes the observable refusal. Two persistent mutations require
that check to go red.

This also narrows D-10 and D-72. A POSIX or WASI descriptor can carry authority
that aligns with a native handle without being that handle. Its userspace
adapter still owns descriptor numbering, preopens, cursor and inheritance
state, and root/cwd policy. Those are COMPAT semantics; their cost does not
justify putting a cursor or a descriptor table into the native ABI.

**Alternatives.** Broaden OWED to include “some related primitive exists” was
rejected because it would continue to equate Read with no Write and WaitQueue
with a futex. An implementation-only classification was rejected because a
correct stub can still expose valuable platform design work. A platform-only
classification was the original mistake: it let a no-op body inherit a claim
from prose. Keeping exact per-function prose without a machine check was
rejected by D-124's rule.

**Consequences.** The host check masks both C++ comment forms, permits only
include directives, discovers active definitions rather than listing their
names, refuses duplicate bodies, requires the historical domain of sixteen,
and compares one source marker per definition with the README register. It
checks BODY evidence with exact compact bodies for the WIRED calls, composed
panic path and one-path stubs. The clock checks pin named source shapes:
validation gates, snapshot order, output stores and return forms. They are not
a general C++ reachability or dataflow proof, and runtime evidence still waits
for the port to compile.

REMAINDER is platform judgment, not something a C++ body can prove; named
kernel witnesses guard the disputed distinctions between explicit-offset Read,
internal Close, missing Write/futexes, composed Panic and thread-only Exit.
D-182 adds the trap-time reclamation witness that narrows Close from DOOR to
PARTIAL, alongside VM unmap and anonymous free.

The ABI check removes comments from both the C header and the Rust sources,
compares the complete C call and opcode domains with the kernel decoders,
checks the errno values the C++ consumes, and derives C submission, completion
and time layouts — offsets, widths and total sizes — before comparing them with
the kernel's access maps. It also inspects the emitted ELF's actual operands for
opcode, token, argument, status and value, plus entry base and count. Another
speculative member, commented-out decoder arm, reordered field or stale program
operand cannot hide after the known members. Aggregate counts are derived facts
and are not restated as authority.

The first check was broken four ways and red each time (D-28): Exit actively
rang the wrong call; Read and Close exchanged remainder labels in both C++ and
README; the clock's 128-bit intermediate narrowed to `u64`; and proposed call
4 returned to the C enum. The two-file label swap had gone red only because
the test contained a third copy of the whole register, violating D-157. That
result was rejected rather than counted: the copied map was removed, named
platform witnesses were added, and the same swap then went red for the
Read/Close evidence it actually contradicted.

The repaired checks were broken thirteen further ways and red: a duplicate
active definition, a body hidden behind `#if 0`, the Exit call hidden in a
block comment, a stub with an extra success return, EINVAL renumbered, the C
clock's first two fields exchanged, a non-literal decoder arm, and the TSC
halves recombined backwards; then a spaced macro directive, a dead conditional
that called Exit, a block-commented errno definition, a block-commented kernel
decoder arm, and a 64-bit C `shift` over the kernel's 32-bit field. These are
host repository-claim checks, not QEMU verdicts, so they do not add a machine
mutation to `cargo xtask mutate`.

The authority and layout repair added seven more observed host reds: the
kernel submission `argument` offset moved from 24 to 8; an eighth opcode entered
the decoder; the OpenAt witness was hidden in a block comment; the clock's
seconds store was commented out; rights intersection became union; and Panic
and Open were each relabelled with the broader remainder their platform witness
contradicts. Six target breaks were then booted and red: OpenAt restored every
universal right, File admitted inert `WRITE`, Directory admitted inert
`MODIFY`, the kernel parsed `argument` at offset 8, and OpenAt read a malformed
path before reserving against a full table; finally Duplicate validated malformed
rights before making the same reservation. The six target defects have
persistent mutations; the then-current register held 188 machine checks and 75
mutations.

No kernel ABI was added and no libc was claimed compiled. Phase 7 slice 2 now
starts from an honest pressure-test transcription; Phase 9 still owns the
pinned mlibc port, POSIX state and signal representation.
