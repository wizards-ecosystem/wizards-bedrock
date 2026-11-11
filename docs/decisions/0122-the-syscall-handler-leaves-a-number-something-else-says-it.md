# D-122 The syscall handler leaves a number; something else says it

Status: Accepted, 2026-08-24.

**Decision.** **A trap handler that wants to say something writes a number, and
a thread says it.** The handler cannot know which locks its caller holds; a
thread can. A refused call increments a counter and stores the number asked
for, and the **idle thread** reports it, in a context where taking the
console's lock is legal. At most once per boot, so a program in a loop cannot
turn refusals into a flood.

**Rationale.** The unknown-call arm answered `-ENOSYS` and then logged, and
logging takes the console's `ThreadLock`, whose
`assert!(!interrupt::in_trap() || is_panicking())` is D-63's way of making a
lock a handler must not take say so. So **`mov rax, 3; int 0x80` from ring 3
halted the machine**: unprivileged, unconditional, and shipped, since the
initrd is untrusted input by D-100. It was green because **no check had ever
issued an unrecognised call number.**

**Consequences.** The program has a third behaviour (argument two asks for
`0x0BAD_0BAD` and then exits), so the arm is executed on every boot, which is
the only reason a check about it can fail.
`an_unserved_call_is_refused_without_stopping_the_machine` reads the *count*,
because "the machine is still here" is established by the check running at all.
Its break panics on the assert itself, which is the defect being a defect.
`cargo xtask mutate` carries it.

**2026-10-16 validation amendment.** Mutation 1,630 changes the one-shot latch
so each later unknown syscall can re-arm synchronous UART output. Its original
machine witness shared aggregate process-retirement state with surrounding
checks and could time out before it observed the mutation. The repaired witness
drains pre-existing zombies within a bound, retains each `Spawned` authority,
waits for that exact process's last rites, issues a second real refusal, and
requires the raw boot-global report latch to remain exactly one before bounded
cleanup. Mutation 2,083 removes cleanup's ownership comparison; a child held
live at the existing ring-entry gate makes that exact break red before it is
unconditionally released and matched to its own last rites. This changes the
witness, not the once-per-boot production policy. Implementation of that
hermetic witness is present; the replacement full sweep is still qualification
work rather than evidence already earned.

The program's new `jz` is patched from its label like every other branch,
because the hand-computed one landed mid-instruction, the failure D-116's
label patching exists to prevent; the encoding test checks where it lands.

`git add -A` in a tree another process is editing is a mistake: it once swept
an audit's in-flight `libs/x86-apic` edit into a commit, leaving `main` red
while the working tree was not.

Left owed: the two other audits from this pass have findings not yet acted on:
`send_self_interrupt` accepting a vector the CPU pushes an error code for,
`reclaim` not consulting `bound`, `unmap` handing back user pages unzeroed, and
a list of checks that cannot fail. Each is named in the audit and is next.
