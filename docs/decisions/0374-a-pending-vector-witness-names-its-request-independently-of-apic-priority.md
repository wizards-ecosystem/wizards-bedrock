# D-374 A pending-vector witness names its request independently of APIC priority

Status: Accepted, 2026-10-09. Amends D-372; closes PW-583.

**Decision.** One shared helper snapshots all eight IRR banks. Production still
selects and reports their highest pending vector before `sti`. The self-test
separately asks whether the exact self-IPI bit is present in that snapshot and
accepts refusal of any pending vector. It still proves the sent request entered
IRR while interrupts were excluded, the door kept them excluded, no delivery
occurred before unwind, and the self-IPI arrived afterward.

**Rationale.** The first complete post-D-373 gate failed only `a pending local
interrupt refuses the first sti`, and an unchanged rerun passed. The
production door correctly read all eight IRR banks and refused the highest
pending vector, but the machine witness declared its queued self-IPI present
only when that vector was itself highest and required the refusal to name it,
so a legitimate timer request at a higher vector could hide the self-IPI from
the test without bypassing the door.

**Alternatives.** Disarming the timer would make the test alter unrelated live
scheduler state. Raising the self-IPI's vector would test an artificial
priority arrangement. Treating any pending bit as proof that the sent self-IPI
queued would permit unrelated work to satisfy the witness. Exact-bit presence
keeps the observation causal without weakening the production refusal.

**Consequences.** Pure host logic finds a named pending vector beneath a
higher-priority request and rejects an absent neighbour. Mutation 1,759
restores the erroneous highest-only interpretation and fails only that host
test.
