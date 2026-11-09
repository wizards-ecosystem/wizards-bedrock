# D-117 A dead program's memory comes back, collected by whoever has nothing better to do

Status: Accepted (2026-08-24)

**Context.** Phase 5 closed with nothing reclaimed. A retired thread kept its
slot, its stack, its address space, its handle table and its ring for the life
of the boot — three programs a boot made that bounded and uninteresting, and it
stops being either the moment something spawns in a loop, which is what an
operating system does. `011`'s close recorded it; this pays it.

**Decision — who collects.** Not the dying thread. It is standing on the kernel
stack it would have to free and running on the tables it would have to reclaim;
any design where it tries is a use-after-free wearing a comment. So a retired
program's resources become a **zombie**, and two other contexts collect: the
**idle thread**, which *is* "the machine has nothing better to do" and is
therefore exactly the right time to free memory, and **spawn**, because a
machine busy enough never to idle would otherwise never collect — and the thing
most likely to need a slot is the thing most likely to free one.

One per pass rather than all of them. A loop that frees sixteen processes with
interrupts on can be preempted halfway through anyway; one at a time makes that
unremarkable instead of something to reason about.

**Decision — what a space owns.** `take_table` forgot every page-table frame,
which is what makes a table reachable through the direct map with no handle
naming it. That was right and the conclusion drawn from it was not: the
kernel's own tables never come back, but a *user* space's should. An
`AddressSpace` now records its own tables — not the shared kernel entries
`for_user` copies, which belong to the kernel — and `reclaim` gives them back.
Past a ceiling of thirty-two the frames are still correct and simply not
returned, **counted rather than silent**, so the leak is a number somebody can
read.

`reclaim` refuses if these are the tables in `CR3`. Freeing the tables under
the register that is walking them is a triple fault at the next memory access,
and it is exactly the mistake a reclaim path makes.

`bind` therefore **borrows** rather than consumes, with a one-shot flag so a
space still binds at most once. The obligation that replaces the old one is
stated and not enforced: the space must outlive every thread bound to it, which
the caller arranges by keeping it beside the thread's other resources and
freeing them together.

**Decision — the order.** Mappings, then the handle table, then the ring, then
the tables. Each step is why the next is possible: the frames go back before
the tables that reach them do; the table's `Drop` releases the bootstrap
channel, so the parent's end learns its peer is gone; the ring's pages are
*not* among the mappings, because `map_shared` wrote no owned bit and `unmap`
would refuse them — they go back when the ring drops.

**Two tables had to agree.** `retire_from_trap` freed the kernel's slot and
`switch_from_trap` left `wost`'s `Alive`, so every later spawn that picked that
index was refused for ever. `task::retire_to` is `switch_from_trap` with the
leaving slot marked **dead**, which is what makes both the slot and its stack
reusable. This is the same disagreement D-115 found on the binding path, on the
path that actually runs.

**An exit record, because a collected process cannot be asked anything.**
`LastRites` is taken at the moment of death — handles held, what the bootstrap
named, completions served, the submission and the completion. Before this,
every check that inspected a retired program was reading a corpse the kernel
had not got round to burying, and those checks broke the instant burial
started. Keeping the record is also just right: "what did that program do" is
worth answering once the program cannot answer it.

**A check's premise changed, and the check had to say something else.**
`a_second_thread_crosses_onto_its_own_stack` asserted that two crossings landed
on *different* stacks. That held because a retired slot was never reused — so
two crossings could not share a stack even if `RSP0` never moved, which makes
it a weaker claim than it looked. Slots are reused now and two threads that
never coexist correctly share one. The property is that a trap lands on the
arena slot belonging to **the thread that made it**, so the entry path records
the calling slot and the check checks that. Strictly stronger, and reuse-proof.

Three other checks compared free-slot counts for equality, which became a race
the moment user threads began freeing their own slots asynchronously. They wait
for `>=` now: the property is that *this* thread's slot came back, and another
coming back with it does not falsify it.

**Consequences.** 126 checks. `a_dead_program_gives_its_memory_back` measures
in **frames**, not slots: a freed slot says a thread ended, which was already
true — the question is whether the memory came back, and the allocator is the
only thing that can answer it. It then runs **forty programs through sixteen
slots** and requires the frame count not to fall, which without reclamation
fails at the seventeenth spawn and would have leaked hundreds of frames before
that. Broken twice and red both times: the unmap removed, and the table
tracking removed.

Left owed: a program's *stack* is still the kernel's boot-time arena rather
than mapped per thread, so `STACK_SLOTS = 16` remains a real cap on
concurrency even though it is no longer a cap on programs ever run. Nothing
reclaims a space with more than thirty-two of its own tables. And a zombie
waits for a collector, so a machine that neither idles nor spawns holds them —
bounded by the slot count, which is the honest version of "eventually".
