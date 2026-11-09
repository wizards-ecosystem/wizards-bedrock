# D-106 A thread's address space is installed by the switch, and the binding is evidence

Status: Accepted, 2026-08-24.

**Context.** D-104 built `AddressSpace::for_user`: a space sharing the kernel's
higher half and empty below it. Nothing installed one. Every program ran on
the kernel's own tables at addresses chosen to avoid each other, which slice 5
was explicit about — *"that is not isolation and is not claimed to be"*.

**Options.** A `CR3` write at the crossing, in `enter_user`; a per-process
field the scheduler consults; or the switch itself.

**Decision.** The switch. `task::prepare` is the one place every context change
passes through, and it already writes `RSP0` per thread for the same reason
(D-99). A slot carries a `page_table`, zero meaning the kernel's, and `prepare`
loads it when it differs from what is in `CR3`.

Installing tables is the most dangerous single instruction in this kernel: a
`CR3` that does not map the code about to execute is a triple fault with no
diagnostic. So the root does not travel as a number. `AddressSpace::bind`
consumes the space, compares **every top-level entry of the higher half**
against the live kernel's, and returns a `Binding` — the evidence type
`task::bind_address_space` demands. One comparison covers the kernel's code,
its stacks, the direct map, the trap tables and the device window, and it is
total where sampling addresses is not.

The binding happens while the new slot is still `Reserved` — after `wost` has a
slot and before anything can schedule it. That window is `spawn_in_space`'s
whole reason to exist: a thread's tables, and whatever its entry point must
find, have to be in place before its first instruction rather than racing it.

**Rationale.** The alternative shapes each put the `CR3` write somewhere that
runs *less* often than a switch, which sounds cheaper and is wrong: a thread
preempted in ring 3 and resumed must come back on its own tables, and only the
switch sees that. Putting it in `prepare` also means the check that it worked
is free — `cross` compares `cpu::page_table_base()` against the root it was
given and refuses rather than writing one program's argument onto another's
stack.

**A third door into the higher half, found by breaking the check.** D-105
closed `map_boot` and `unmap` against a space that shares the kernel.
`map_device` was not closed, and the device window is in the higher half: a
user space could walk into the kernel's own device tables and edit them. It
takes the same refusal now. This was found because the break-test for `bind`'s
comparison compiled clean — nothing checked it — and writing the check that
would have caught it turned up the door.

**Consequences.** `load_image`, `place_stack` and `undo` take the space they
map into rather than reaching for the running one, so a program is loaded into
tables no other thread walks. Two checks that read a program's pages from a
kernel thread had to stop: since this decision no program is in the running
space. They map their own probe page instead, which is the honest arrangement
— what they test is the copy and the walk, not the loader.

The isolation claim is now the strong one, and checked: two spaces, one virtual
address, two frames, and the same kernel in both. `two_spaces_map_one_address_to_different_memory`
is in the gate at 110 checks.

Left owed. An `AddressSpace` still has no `Drop` — the deliberate path is
`reclaim`, which D-117 added and the zombie collector calls, so a *retired
program's* space is given back. A space built and **abandoned** without that
call still costs its table frames, and so does a half-loaded program's memory.
Bounded today (three programs and a handful of refusals per boot) and named
here rather than carried quietly; D-44's accounting is where it is settled.
~~Nothing yet unmaps a retired thread's space~~ — **closed by D-117** — and
`APART` still separates two programs' addresses where it no longer needs to.
