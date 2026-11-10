# D-117 A dead program's memory comes back, collected by whoever has nothing better to do

Status: Accepted, 2026-08-24.

**Context.** Phase 5 closed with nothing reclaimed. A retired thread kept its
slot, stack, address space, handle table and ring for the life of the boot.
Three programs a boot made that bounded and uninteresting, until something
spawns in a loop, which is what an operating system does. `011`'s close recorded
the debt; this pays it.

**Decision: who collects.** Not the dying thread. It stands on the kernel stack
it would have to free and runs on the tables it would have to reclaim, so any
design where it tries is a use-after-free. A retired program's resources become
a **zombie**, and two other contexts collect. The **idle thread** *is* "the
machine has nothing better to do", the right time to free memory. **Spawn**
collects too, because a machine too busy to idle would otherwise never collect,
and the thing most likely to need a slot is the thing most likely to free one.

Each pass collects one zombie. A loop freeing sixteen processes with interrupts
on can be preempted halfway anyway; one at a time makes that unremarkable.

**Decision: what a space owns.** `take_table` forgot every page-table frame,
which is what makes a table reachable through the direct map with no handle
naming it. That was right, but the conclusion drawn from it was wrong: the
kernel's own tables never come back, and a *user* space's should. An
`AddressSpace` now records its own tables, excluding the shared kernel entries
`for_user` copies, and `reclaim` gives them back. Past a ceiling of thirty-two,
the frames stay correct and are not returned, **counted rather than silent**, so
the leak is a number somebody can read.

`reclaim` refuses if these are the tables in `CR3`. Freeing the tables the
register is walking is a triple fault at the next memory access, the typical
mistake of a reclaim path.

`bind` therefore **borrows** instead of consuming, with a one-shot flag so a
space still binds at most once. The replacement obligation is stated and not
enforced: the space must outlive every thread bound to it. The caller keeps it
beside the thread's other resources and frees them together.

**Decision: the order.** Mappings, then the handle table, then the ring, then
the tables. Each step enables the next. The frames go back before the tables
that reach them. The table's `Drop` releases the bootstrap channel, so the
parent's end learns its peer is gone. The ring's pages are *not* among the
mappings, because `map_shared` wrote no owned bit and `unmap` would refuse them;
they go back when the ring drops.

**Two tables had to agree.** `retire_from_trap` freed the kernel's slot and
`switch_from_trap` left `wost`'s `Alive`, so every later spawn of that index was
refused for ever. `task::retire_to` is `switch_from_trap` with the leaving slot
marked **dead**, which makes both the slot and its stack reusable. D-115 found
the same disagreement on the binding path; this is the path that runs.

**An exit record, because a collected process cannot be asked anything.**
`LastRites` is taken at death: handles held, what the bootstrap named,
completions served, the submission and the completion. Before this, checks of a
retired program read a corpse the kernel had not yet buried, and they broke once
burial started. "What did that program do" stays answerable after the program is
gone.

**A check's premise changed, so the check had to say something else.**
`a_second_thread_crosses_onto_its_own_stack` asserted that two crossings landed
on *different* stacks. That held because a retired slot was never reused, so two
crossings could not share a stack even if `RSP0` never moved. The claim was
weaker than it looked. Slots are reused now, and two threads that never coexist
correctly share one. The property is that a trap lands on the arena slot of
**the thread that made it**, so the entry path records the calling slot and the
check checks that. It is strictly stronger and survives reuse.

Three other checks compared free-slot counts for equality, which became a race
once user threads freed their own slots asynchronously. They now wait for `>=`:
the property is that *this* thread's slot came back, and another coming back
with it does not falsify it.

**Consequences.** 126 checks. `a_dead_program_gives_its_memory_back` measures in
**frames**, not slots. A freed slot says a thread ended, which was already true;
whether the memory came back only the allocator can answer. It then runs **forty
programs through sixteen slots** and requires the frame count not to fall.
Without reclamation it fails at the seventeenth spawn, after leaking hundreds of
frames. Broken twice and red both times: the unmap removed, and the table
tracking removed.

Left owed: a program's *stack* is still the kernel's boot-time arena, not mapped
per thread. `STACK_SLOTS = 16` remains a cap on concurrency, though no longer on
programs ever run. Nothing reclaims a space with more than thirty-two of its own
tables. A zombie waits for a collector, so a machine that neither idles nor
spawns holds them, bounded by the slot count.
