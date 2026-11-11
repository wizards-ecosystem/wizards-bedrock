# D-116 A program drives its own ring, and the shipped kernel runs one

Status: Accepted, 2026-08-24. Amended by D-181.

**Decision: the gate dispatches.** `rax` selects, and **only `exit` does not
return**. There are two calls, and the surface stays small because io_uring's
exploit record warns about surface growth. An unknown number is refused *with an
answer* (`-ENOSYS`), never ignored. A program that gets nothing back cannot tell
a wrong call from a dropped one, and would wait for ever.

Returning a value needs a writable trap frame, and `TrapFrame` is a read-only
snapshot by design. So there is exactly one writer: `interrupt::answer` sets
`rax` and nothing else, and refuses unless the frame is the innermost live one.
`switch_from_trap` makes the same refusal for the same reason: a stale
snapshot's `origin` names a stack another thread now stands on.

**Decision: admission happens at the doorbell.** `serve` reserves as it goes. A
program following the ABI (write an entry, move the tail, ring the doorbell)
must be served without a separate `admit` call. The reservation is still taken
**before** the operation is accepted, which is D-79's rule and what D-112's
model checks. A submission with nowhere to report stays unserved instead of
being served and dropped, which is why `serve` can stop early with entries
still published. `reserve_for_test` exercises admission without a program; its
name marks that using it with a published tail would double-count and is not
the ABI.

**Decision: the program is emitted, not transcribed.** A hand-assembled listing
gets a displacement wrong silently: the branch lands mid-instruction and the
program *runs*, doing something else. So `xtask` emits it and patches the jumps
from labels, and a build-time `expect` catches a displacement too far. A host
test pins the encoding: the ring's address against `user::RING`, the two
`int 0x80`s in order, and every branch landing on the `ud2`.

The program checks its own answer: exactly one served, the value echoed back, a
clean status. Every failure is a `ud2`, so a wrong answer is a fault, never a
quiet pass.

**Decision: the shipped kernel runs it.** The syscall path reaches the
userspace modules unconditionally. **A kernel that has userspace and never
enters it has not shipped userspace**, so gating the *caller* would have been
the wrong repair. `boot-check` covers the whole path.

What is not shipped yet carries `#[allow(dead_code)]` on the impl, not the
module, each naming the owed syscall, so a dead item still shows.

**Rationale.** `002` pillar 2, that the native data-path ABI is
submission/completion queues, was a claim about code nothing could use until a
program could cross and come back. The first doorbell version served a program
nothing while every check passed, because the checks were written against the
kernel's shape, not a program's.

**Consequences.** `a_program_drives_its_own_ring` transcribes nothing. It reads
back the submission the *program* wrote and compares the completion against
**that**, so a check cannot agree with a constant it also chose. The kernel
never posts to a program's ring, so a match proves a program ran. Its tally is
kernel-side, because the program's own lives in a page the program can write
(D-115).

At the time, the ring carried no handle, nothing blocked on a completion, and
the eleven entries OWED under D-114's former taxonomy stayed owed after D-118
paid `sys_tcb_set`. D-181 retires that taxonomy. This decision's live result is
the two calls the emitted program needed to prove the ABI worked, not a complete
libc surface.
