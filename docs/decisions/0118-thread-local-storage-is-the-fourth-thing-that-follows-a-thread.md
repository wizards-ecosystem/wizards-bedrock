# D-118 Thread-local storage is the fourth thing that follows a thread

Status: Accepted (2026-08-24) (amended by D-181)

**Context.** D-114 wrote mlibc's sixteen mandatory sysdeps against the native
ABI and named four things the exercise surfaced that nothing native would have
raised. The first was that `sys_tcb_set` writes `FS_BASE` and the kernel does
not save it across a context switch — a hole in a place three neighbouring
decisions had already visited, and one only a POSIX consumer would have looked
for, because nothing native has thread-local storage.

**Decision.** `FS_BASE` joins the privilege stack (D-99), the tables (D-106)
and the extended state (D-108) in `task::prepare` — the one place every switch
passes through, which is now the whole answer to "what follows a thread". Two
MSR accesses on a path that already writes two control registers.

Saved from the *register* rather than from a remembered value, because ring 3
can change it: a program that set its own would otherwise have the kernel
restore the one it was given.

**The safe door range-checks, and against the user boundary rather than
canonicality.** `wrmsr` to `FS_BASE` with a non-canonical value is a general
protection fault *in kernel context* — a program crashing the handler rather
than itself — so `set_thread_base` refuses first and writes second. It refuses
the kernel's half as well, which canonicality alone would allow: a program
asking to point `fs:` at kernel memory has no reason it could be given, and on
this architecture the two checks are one because the boundary is where the
canonical hole begins.

`paging::is_user_address` is that check, and it is a **range comparison, never
`>> 47 == 0`** (D-84): the shift form encodes an assumption about how many bits
the machine walks, which five-level paging changes.

**Consequences.** `WIZ_CALL_TCB_SET` is the third implemented call, and the ABI
header's numbering moved with it — the owed calls renumbered rather than
leaving a gap, because a gap is a number somebody eventually fills wrongly.
`sys_tcb_set` moves from OWED to MAPS: four map now, eleven are owed, one stays
absent. These are D-114's historical labels; D-181 now records
`sys_tcb_set` as BODY=WIRED, REMAINDER=WORKS.

127 checks. `thread_local_storage_follows_the_thread` sets a base, has a second
thread set its own, and requires each to keep what it set — which is only true
if the switch saves and restores. Broken twice and red both times: the restore
removed, and the range check removed.

Left owed: `GS_BASE` and `swapgs`, which are still not paid and still have no
caller that would benefit on one CPU — the phase close said Phase 12 with SMP,
and this decision does not change that. A thread's base is not inherited by
anything, because nothing yet creates a thread inside an existing process.
