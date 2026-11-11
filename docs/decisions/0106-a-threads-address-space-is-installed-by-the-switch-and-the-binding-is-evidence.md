# D-106 A thread's address space is installed by the switch, and the binding is evidence

Status: Accepted, 2026-08-24.

**Decision.** The switch installs a thread's address space. `task::prepare` is
the one place every context change passes through, and it already writes
`RSP0` per thread for the same reason (D-99). A slot carries a `page_table`,
zero meaning the kernel's, and `prepare` loads it when it differs from `CR3`.

The root does not travel as a number, because a `CR3` that does not map the
code about to execute is a triple fault with no diagnostic.
`AddressSpace::bind` consumes the space, compares **every top-level entry of the
higher half** against the live kernel's, and returns a `Binding`, the evidence
type `task::bind_address_space` demands. That one total comparison covers the
kernel's code, stacks, direct map, trap tables and device window; sampling
addresses would not.

The binding happens while the new slot is still `Reserved`, before anything can
schedule it. That window is why `spawn_in_space` exists: a thread's tables and
whatever its entry point must find have to be in place before its first
instruction, not racing it.

`map_device` refuses a space that shares the kernel, as D-105 made `map_boot`
and `unmap` do, because the device window is in the higher half.

**Rationale.** A `CR3` write at the crossing or a per-process field the
scheduler consults would run *less* often than a switch, which sounds cheaper
and is wrong: only the switch sees a thread preempted in ring 3 resume, and it
must resume on its own tables. `cross` compares `cpu::page_table_base()`
against the root it was given, and refuses instead of writing one program's
argument onto another's stack.

**Consequences.** `load_image`, `place_stack` and `undo` take the space they map
into instead of the running one, so a program loads into tables no other thread
walks. The isolation claim is checked:
`two_spaces_map_one_address_to_different_memory` maps one virtual address to
two frames with the same kernel in both.

Left owed: an `AddressSpace` still has no `Drop`. The intended path is
`reclaim`, which D-117 added and the zombie collector calls, so a *retired
program's* space is given back. A space built and **abandoned** without that
call still costs its table frames, as does a half-loaded program's memory. This
is bounded today (three programs and a handful of refusals per boot) and named here rather than carried quietly; D-44's accounting
is where it is settled.
~~Nothing yet unmaps a retired thread's space~~ (**closed by D-117**), and
`APART` still separates two programs' addresses where it no longer needs to.
