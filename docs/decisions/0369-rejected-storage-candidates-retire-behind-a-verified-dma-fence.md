# D-369 Rejected storage candidates retire behind a verified DMA fence

Status: Accepted, 2026-10-09. Amends D-199, D-335, D-357 and D-358.

**Context.** Deep Codex Security scan
`32245094-b8b7-47d1-b6e7-a52554800772` found three consequences of one split
NVMe teardown. A clean `Controller::retire` dropped admin and I/O queue frames
without first removing their VT-d leaves. The later PCI release restored the
complete firmware command word, including an inherited Bus Master Enable bit,
before the candidate's VT-d domain switched off. A hostile controller could
therefore keep using recycled queue frames through stale translations, or
regain unrestricted DMA after translation disappeared. The same failure path
reported live VT-d faults in an unbounded loop before attempting retirement,
so a device which replenished fault records could prevent the fence from ever
being reached. These are occurrences `occ_59cd8640eb7bae0db2efdeac`,
`occ_9389119de87ecd531d2e1b7f`, `occ_e1509723e924ca0523f5334a`
and `occ_431d1817e808a2ca0790ad14`.

D-358 deliberately left queue mappings installed until domain release and
allowed the captured firmware word to restore BME after a verified controller
stop. That is sufficient for a cooperative device, but not for the repository's
hostile-device boundary: `CC.EN = 0` and `CSTS.RDY = 0` are values supplied by
the same endpoint whose future DMA must be constrained. D-199's exact transient
restoration likewise needs an explicit exception for a rejected DMA-capable
function. Returning every other captured bit remains compatible; re-enabling
unconfined DMA is not.

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

**Proof required.** Pure PCI tests must begin with inherited BME set, exercise
successful, refused, interrupted and retry transitions, and prove final restore
equals the captured word with only BME removed. Source contracts must bind the
transition to exact function authority, prove BME clearance precedes queue
unmapping, prove both queue segments reach the invalidating boundary, prove
mapped queue segments cannot drop recyclable frames, and bound production
fault draining. Existing partial-map, command-linearity, candidate-baseline and
degraded-storage witnesses must remain green. Every new check needs an
independently observed retained red mutation before release/debug boots and the
ordinary gate pass.

**Evidence.** On 2026-10-09 the PCI host suite exercised inherited BME,
foreign function authority, all four retirement access-failure positions,
interrupted rollback, retry and safe final restoration. DMA source contracts
proved that the controller owns a new I/O queue before mapping, that successful
and uncertain maps conservatively mark both queue segments, that only
successful invalidation clears those facts, and that `Drop` retains each still
reachable segment. The fault contract bound the loop operands and its one-step
budget consumption as well as retirement-before-reporting order. Retained
mutations 1,651 through 1,664 were each observed red against their exact named
witness before the pristine focused suites were rerun. The ordinary
`cargo xtask ci` gate then passed: all 1,923 public API items had
compiler-resolved callers, 723 active xtask tests and all doctests passed, the
supply-chain inventory covered 72 registry crates, three vendored artifacts and
four runtime inputs, the release machine passed 275/275 self-tests, shipped
liveness responded, and the optional-protection and no-remapping degraded boots
both settled.

**Alternatives.** Swapping PCI and VT-d release would switch translation off
while an admitted controller still had BME. Queue unmapping alone would leave
the rejected function unrestricted after domain release. BME clearing alone
would keep stale leaves pointed at recycled frames if later configuration or
domain cleanup refused. Restoring firmware's BME after teardown would recreate
the finding. Moving the large `Vtd` value through candidate stack frames would
undo D-359; the existing installed slot remains the linear domain owner.
