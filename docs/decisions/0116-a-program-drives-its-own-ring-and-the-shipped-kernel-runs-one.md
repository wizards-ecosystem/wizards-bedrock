# D-116 A program drives its own ring, and the shipped kernel runs one

Status: Accepted (2026-08-24) (amended by D-181)

**Context.** Phase 5 closed with the async ABI complete on the kernel's side
and unreachable from ring 3. The gate at DPL 3 retired its caller
unconditionally, so a program could cross exactly once and never act on what it
learned; the shared pages, the admission, the terminal completion were all
exercised only by kernel checks. `011`'s close named it the largest thing owed,
and `002` pillar 2 — the native data-path ABI is submission/completion queues —
was a claim about code nothing could use.

**Decision — the gate dispatches.** `rax` selects, and **only `exit` does not
return**. Two calls, and the surface stays small on purpose: io_uring's exploit
record is a warning about surface growth. An unknown number is refused *with an
answer* (`-ENOSYS`), never ignored — a program that gets nothing back cannot
tell a call it made wrong from one the kernel dropped, and would wait for ever
on the difference.

Returning a value needs a writable trap frame, and `TrapFrame` is a read-only
snapshot on purpose. So there is exactly one writer: `interrupt::answer` sets
`rax` and nothing else, and refuses unless the frame is the innermost live one
— the same refusal `switch_from_trap` makes, for the same reason. A stale
snapshot's `origin` names a stack another thread is now standing on.

**Decision — admission happens at the doorbell.** This was wrong first, and the
way it was wrong is worth keeping. The kernel reserved completion slots through
a separate `admit` call, and `serve` took whatever had been reserved. A program
doing exactly what the ABI describes — write an entry, move the tail, ring the
doorbell — was served *nothing*, because the kernel's own ring never learned
about entries it had not admitted. The checks passed throughout: every one of
them called `admit` first, because they were written against the kernel's
shape rather than a program's.

`serve` now reserves as it goes. The reservation is still taken **before** the
operation is accepted, which is the whole of D-79 and what D-112's model
checks: a submission with nowhere to report is left unserved rather than served
and dropped. That is the backpressure, and it is why `serve` can stop early
with entries still published. `reserve_for_test` is what the admission checks
use to exercise it without a program in the way, and it is named so that using
it and publishing a tail — which would double-count — is visibly not the ABI.

**Decision — the program is emitted, not transcribed.** Driving a ring takes
about a hundred bytes of instructions with three conditional branches, and a
hand-assembled listing gets a displacement wrong silently: the branch lands in
the middle of an instruction and the program *runs*, doing something else. So
`xtask` emits it and patches the jumps from labels, a build-time `expect`
catches a displacement too far, and a host test pins the encoding — the ring's
address against `user::RING`, the two `int 0x80`s in order, and every branch
landing on the `ud2`.

The program checks its own answer: exactly one served, the value echoed back,
a clean status. Every failure is a `ud2`. So the program and the check disagree
in the same direction, and a wrong answer is a fault rather than a quiet pass.

**Decision — the shipped kernel runs it.** The userspace modules were gated to
the selftest feature; the syscall path reaches them unconditionally now, and
gating the *caller* instead would have been the wrong repair. **A kernel that
has userspace and never enters it has not shipped userspace** — the loader, the
address space, the handle table, the ring and the gate were dead code in the
image people actually boot. `boot-check` now covers the whole path.

What is genuinely not shipped yet is marked `#[allow(dead_code)]` on the impl
rather than the module, each with the owed syscall named, so a genuinely dead
item still shows.

**Consequences.** 125 checks. `a_program_drives_its_own_ring` transcribes
nothing: it reads back the submission the *program* wrote and compares the
completion against **that**, so a check agreeing with a constant it also chose
is not expressible. The kernel never posts to a program's ring, so a matching
pair can only have come from a program that ran. The tally it counts is
kernel-side, because the program's own is in a page the program can write
(D-115).

Broken twice and red both times: the doorbell answering zero, and the echo
returning the wrong word — the second caught by the program *and* the kernel
independently, which is what the doubled check buys.

The check-count cross-check D-105 added earned its place immediately: bumping
`EXPECTED_CHECKS` without `EXPECTED_SELFTEST_CHECKS` turned `cargo test` red in
a second, where before it would have cost a sixty-second boot to find.

At the time, the ring carried no handle, nothing blocked on a completion, and
the eleven entries OWED under D-114's former taxonomy remained so after D-118
paid `sys_tcb_set`. D-181 retires that taxonomy; this decision's live result is
that it added the two calls the emitted program needed to prove the ABI worked,
not a complete libc surface.
