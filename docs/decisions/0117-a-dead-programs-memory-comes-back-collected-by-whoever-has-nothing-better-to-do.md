# D-117 A dead program's memory comes back, collected by whoever has nothing better to do

Status: Accepted, 2026-08-24.

**Decision: who collects.** Not the dying thread. It stands on the kernel stack
it would have to free and runs on the tables it would have to reclaim, so any
design where it tries is a use-after-free. A retired program's resources become
a **zombie**, and two other contexts collect. The **idle thread** *is* "the
machine has nothing better to do", the right time to free memory. **Spawn**
collects too, because a machine too busy to idle would otherwise never collect,
and the thing most likely to need a slot is the thing most likely to free one.

Each pass collects one zombie. A loop freeing sixteen processes with interrupts
on can be preempted halfway anyway; one at a time makes that unremarkable.

**Decision: what a space owns.** The kernel's own tables never come back, and a
*user* space's should. An `AddressSpace` records its own tables, excluding the
shared kernel entries `for_user` copies, and `reclaim` gives them back. Past a
ceiling of thirty-two, the frames stay correct and are not returned, **counted
rather than silent**, so the leak is a number somebody can read.

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

**Two tables agree.** `task::retire_to` is `switch_from_trap` with the leaving
slot marked **dead**, which makes both the kernel's slot and `wost`'s, and its
stack, reusable; otherwise every later spawn of that index is refused for ever.

**An exit record, because a collected process cannot be asked anything.**
`LastRites` is taken at death: handles held, what the bootstrap named,
completions served, the submission and the completion. "What did that program
do" stays answerable after the program is gone.

**Rationale.** Something that spawns in a loop is what an operating system
does, so a retired thread keeping its slot, stack, address space, handle table
and ring for the life of the boot stops being bounded.

**Consequences.** A trap lands on the arena slot of **the thread that made
it**: the entry path records the calling slot and the check checks that, since
reused slots mean two threads that never coexist correctly share one stack.
Free-slot checks wait for `>=`: the property is that *this* thread's slot came
back.

`a_dead_program_gives_its_memory_back` measures in **frames**, not slots,
because whether the memory came back only the allocator can answer. It runs
more programs than there are slots and requires the frame count not to fall.

Left owed: a program's *stack* is still the kernel's boot-time arena, not mapped
per thread. `STACK_SLOTS = 16` remains a cap on concurrency, though no longer on
programs ever run. Nothing reclaims a space with more than thirty-two of its own
tables. A zombie waits for a collector, so a machine that neither idles nor
spawns holds them, bounded by the slot count.
