# D-118 Thread-local storage is the fourth thing that follows a thread

Status: Accepted, 2026-08-24. Amended by D-181.

**Decision.** `FS_BASE` joins the privilege stack (D-99), the tables (D-106) and
the extended state (D-108) in `task::prepare`. Every switch passes through it,
so it answers "what follows a thread", at two MSR accesses on a path that
already writes two control registers. D-114's first finding was that
`sys_tcb_set` writes `FS_BASE` and the kernel did not save it.

It is saved from the *register*, not a remembered value, because ring 3 can
change it, and a remembered value would undo a program's own setting.

**The safe door range-checks against the user boundary, not canonicality.**
`wrmsr` to `FS_BASE` with a non-canonical value is a general protection fault
*in kernel context*, a program crashing the handler instead of itself. So
`set_thread_base` refuses first and writes second. It also refuses the kernel's
half, which canonicality alone would allow: a program has no reason to point
`fs:` at kernel memory. On this architecture the two checks are one, because the
boundary is where the canonical hole begins.

`paging::is_user_address` is that check, and it is a **range comparison, never
`>> 47 == 0`** (D-84). The shift form assumes how many bits the machine walks,
which five-level paging changes.

**Rationale.** Only a POSIX consumer would look here, since nothing native has
thread-local storage; the switch is where every per-thread register already
moves.

**Consequences.** `WIZ_CALL_TCB_SET` is the third implemented call. The owed
calls renumbered instead of leaving a gap, because somebody eventually fills a
gap wrongly. `sys_tcb_set` moves from OWED to MAPS: four map now, eleven are
owed, one stays absent. These are D-114's historical labels; D-181 now records
`sys_tcb_set` as BODY=WIRED, REMAINDER=WORKS.

`thread_local_storage_follows_the_thread` requires two threads each to keep the
base it set, which holds only if the switch saves and restores.

Left owed: `GS_BASE` and `swapgs`, still unpaid and without a caller that would
benefit on one CPU; the phase close said Phase 12 with SMP, unchanged. A
thread's base is inherited by nothing, because nothing yet creates a thread
inside an existing process.
