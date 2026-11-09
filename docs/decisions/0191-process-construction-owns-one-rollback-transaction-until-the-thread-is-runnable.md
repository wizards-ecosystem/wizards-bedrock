# D-191 Process construction owns one rollback transaction until the thread is runnable

Status: Accepted (2026-08-27), amends D-100, D-106, D-115 and D-117; closes PW-103 and PW-109

**Context.** `user::spawn` created an `AddressSpace` and then used `?` through
image, stack, time-page, ring, binding, channel, handle-table and thread
construction. `AddressSpace` deliberately has no automatic `Drop`: only the
process owner knows which owned mappings must be removed before its tables.
Consequently every refusal after the root existed abandoned at least that root;
later refusals also lost image and stack frames, shared-mapping keepalives,
ring pages and table ownership. A `Bootstrap` assembled before
`thread::spawn_in_space` had the same problem when slot fabrication or binding
failed.

The loader had a related ownership gap. Successful placements were published
through the global `PLACED` record and taken by the next `load` call, so a
second or re-entrant construction could replace another call's cleanup data.
The ELF entry was checked only after every segment had been mapped and
`PLACED` written; an invalid entry therefore returned an error whose mappings
no caller could identify. While driving the thread failure edges, the second
fabrication also faulted at address `-24`: `task::retire` replaced the slot with
`Slot::VACANT`, erasing the arena stack top which `fabricate` promises to reuse.

**Decision.** `ProcessConstruction` owns the private address space and the
exact owned ranges mapped into it. Its destructor unmaps those ranges, asks the
task table for release evidence and reclaims every table. The transaction
survives through `thread::spawn_in_space`; only its reserved-slot `arrange`
callback converts it into the `Bootstrap` and `Launch` records immediately
before the scheduler marks the thread ready. All channel, table and ring values
remain ordinary owned locals captured beside the transaction, so their normal
destructors cover every earlier refusal. The callback borrows rather than owns
the prepared transaction: if publication refuses, thread construction retires
the bound task first, and only then can the caller drop and reclaim its tables
with release evidence.

Thread construction exposes three machine-only refusal points after scheduler
reservation, task fabrication and address-space binding. Its arrangement
callback can also refuse publication before the slot becomes runnable. Each
rollback returns both scheduler and task state; the task-table retirement path
preserves the slot's immutable arena stack top while clearing its live state
and binding.

`load_image` now returns a `LoadedImage` containing its entry and exact
placements. The entry is checked during preflight and must lie within the
unrounded memory extent of an executable load segment before the first frame is
taken. There is no global last-load ownership record.

**Rationale.** One linear owner makes the commit boundary visible in the type
and makes ordinary `?` correct. Giving `AddressSpace` a general destructor
cannot remove owned leaf mappings it does not enumerate and cannot prove a
bound task released the root. Scattering cleanup across every error arm would
duplicate ownership transitions whose order must remain explicit.
Returning placements makes concurrent constructions independent by value; a
lock around global last-call state would serialize calls without giving the
right caller durable ownership.

**Alternatives.** Keeping the bounded leak was the defect. Preallocating every
resource before the address space would still leave mapping and thread binding
as fallible steps and would hold scarce objects longer. Making the thread
runnable and then asking it to self-destruct on construction failure cannot
reclaim the stack or tables it is running on. Validating the entry only at the
ring-3 crossing remains safe against execution but is too late to make loading
fail-atomic.

**Consequences.** `every refused spawn returns every resource` stops after each
of fourteen construction edges twice, warming retained allocator backing on the
first pass and requiring an exact second-pass baseline for physical frames,
live heap allocations, returned page/table tallies, handle slots and arena
commitment, channel ends, rings, scheduler slots and process records. It
includes reserved, fabricated and bound task states plus refused launch-record
publication. The loader check now requires an invalid entry to refuse without
changing the frame count and keeps two successful loads' returned placement
records independent.

Skipping transaction table reclamation or the scheduler release after refused
arrangement reddened only the resource-ledger check; skipping entry preflight
reddened only the loader check; restoring the stack-erasing retirement
reproduced the exact pre-verdict page fault. They are mutations 96 through 99.
The first full debug boot then overflowed a guarded kernel stack in a nested
3.9 KiB `finish` return frame; committing the completed values directly into
the reserved slot removed that by-value return, and the same debug image passed
all 196 checks. No unsafe operation was added. The image now carries 196
machine checks.
