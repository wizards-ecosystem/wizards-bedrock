# D-122 The syscall handler leaves a number; something else says it

Status: Accepted, 2026-08-24.

**Context.** D-116 gave the gate a dispatcher and made the shipped kernel run a
program. The unknown-call arm answered `-ENOSYS` and then logged the refusal.
Logging takes the console's `ThreadLock`, whose first line is
`assert!(!interrupt::in_trap() || is_panicking())` — the assert D-63 put there
so that a lock a handler must not take says so rather than deadlocking.

So **`mov rax, 3; int 0x80` from ring 3 halted the machine.** Unprivileged,
unconditional, no recovery, and shipped — the initrd is untrusted input by
D-100, and D-116 put a program in the boot image people actually run.

It was green because **no check had ever issued an unrecognised call number.**
Every check drove the two calls that exist. The arm had never executed.

It also falsified three comments at once, all written in the same commit:
`user::call`'s "nothing here may allocate, block, **or take a lock a handler
cannot**", `trap.rs`'s "**Nothing here logs**", and D-116's own "refused *with
an answer*, never ignored" — describing a path that halts before the program
can read the answer.

**Decision.** The handler leaves a number; something else says it. The refusal
increments a counter and stores the number asked for, and the **idle thread**
reports it — a kernel thread, in a context where taking the console's lock is
legal. At most once per boot, so a program in a loop cannot turn refusals into
a flood.

That is the general shape and worth stating as one: **a trap handler that
wants to say something writes a number, and a thread says it.** The handler
cannot know which locks its caller holds; a thread can.

**Consequences.** 129 checks. The program gained a third behaviour — argument
two asks for `0x0BAD_0BAD` and then exits — so the arm is now executed on every
boot, which is the only reason a check about it can fail. `an_unserved_call_is_refused_without_stopping_the_machine`
reads the *count*, because "the machine is still here" is established by the
check running at all.

Broken once, and the break panics at `sync.rs:406` — the assert itself, which
is the defect being a defect. `cargo xtask mutate` carries it.

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

**And the program's own jump was hand-computed and wrong.** Adding the third
behaviour meant a new `jz`, and computing its displacement by hand put it two
bytes past the target and into the middle of an instruction — where it
assembled, ran, and did something else. That is precisely the failure D-116
built the label-patching machinery to prevent, made by the person who built it,
in the same file. The `jz` is patched from its label now like every other
branch, and the encoding test checks where it lands.

**A second finding, from the same audit, about this session's own process.**
An adversarial audit was mutating `libs/x86-apic` to find survivors when a
`git add -A` here swept its in-flight edit and its `.bak` into a commit.
`BOOTSTRAP` shipped as `1 << 9` with its own doc comment one line above still
saying bit 8, so `main` was red while the working tree was not. `git add -A` in
a tree another process is editing is the mistake, and it is recorded here
because the same reflex will be available next time.

Left owed: the two other audits from this pass have findings not yet acted on —
`send_self_interrupt` accepting a vector the CPU pushes an error code for,
`reclaim` not consulting `bound`, `unmap` handing back user pages unzeroed, and
a list of checks that cannot fail. Each is named in the audit and is next.
