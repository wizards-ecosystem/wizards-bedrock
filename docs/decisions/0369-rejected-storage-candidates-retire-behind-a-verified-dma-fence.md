# D-369 Rejected storage candidates retire behind a verified DMA fence

Status: Accepted, 2026-10-09. Amends D-199, D-335, D-357 and D-358.

**Decision.** A rejected storage lease uses a function-bound PCI command
transaction which clears BME and verifies exact readback while retaining the
opaque `CommandState`. Only after that proof may the transaction remove BME
from its captured restoration target. Interrupted or refused transitions keep
the same linear command and function authority retryable; uncertainty retains
the device and stops selection. Generic PCI clients keep exact command
restoration. Successful selected storage still commits its admitted command.

Controller retirement clears BME before treating controller-supplied stop
status as useful evidence. Each queue pair records which of its two segments
may still be installed. The controller stores a new queue owner before its
first fallible mapping transition, so partial-map uncertainty remains in the
retirement inventory rather than falling into a frame-only leak. Retirement
removes and invalidates every such segment
through the installed VT-d owner before unmapping the BAR or dropping any
recyclable queue frame. A partial map or unmap preserves the exact conservative
state: an uncertain segment is permanently retained, and the controller,
device and still-enabled domain stay quarantined while selection halts.
`QueuePair::Drop` is defense in depth and retains any segment still marked
mapped; it never attempts fallible hardware cleanup silently.

The final failed-candidate transaction re-proves BME off even for failures
which occurred before a controller or queue existed, restores every captured
command bit except BME, releases the function, and switches translation off
last. Thus a clean controller may be dropped before final candidate release
only after both the BME fence and every queue invalidation succeeded. A PCI or
VT-d release refusal leaves no recyclable stale queue owner and cannot restore
BME.

VT-d fault reporting drains at most the advertised fault-record inventory,
releases the domain lock, and emits one aggregate diagnostic. Read-failure
retirement runs before this bounded diagnostic, so attacker-controlled live
MMIO cannot postpone the BME fence or quarantine.

**Rationale.** Deep Codex Security scan
`32245094-b8b7-47d1-b6e7-a52554800772` found three consequences of one split
NVMe teardown, occurrences `occ_59cd8640eb7bae0db2efdeac`,
`occ_9389119de87ecd531d2e1b7f`, `occ_e1509723e924ca0523f5334a`
and `occ_431d1817e808a2ca0790ad14`: a clean `Controller::retire` dropped queue
frames before removing their VT-d leaves, PCI release restored an inherited
Bus Master Enable bit before the domain switched off, and an unbounded fault
report loop could keep the fence from being reached. D-358's installed queue
mappings and BME restoration after a verified stop suffice for a cooperative
device, not a hostile one: `CC.EN = 0` and `CSTS.RDY = 0` are values supplied
by the same endpoint whose future DMA must be constrained. D-199's exact
transient restoration likewise needs an explicit exception for a rejected
DMA-capable function; returning every other captured bit remains compatible,
re-enabling unconfined DMA is not.

**Alternatives.** Swapping PCI and VT-d release would switch translation off
while an admitted controller still had BME. Queue unmapping alone would leave
the rejected function unrestricted after domain release. BME clearing alone
would keep stale leaves pointed at recycled frames if later configuration or
domain cleanup refused. Restoring firmware's BME after teardown would recreate
the finding. Moving the large `Vtd` value through candidate stack frames would
undo D-359; the existing installed slot remains the linear domain owner.

**Consequences.** The existing partial-map, command-linearity,
candidate-baseline and degraded-storage witnesses stay green. Host tests and
source contracts cover inherited BME, foreign function authority, all four
retirement access-failure positions, interrupted rollback, retry, safe final
restoration, BME clearance before queue unmapping, both queue segments reaching
the invalidating boundary, `Drop` retention and bounded production fault
draining. Retained mutations 1,651 through 1,664 each fail their exact named
witness.
