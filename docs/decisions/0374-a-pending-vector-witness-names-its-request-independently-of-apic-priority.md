# D-374 A pending-vector witness names its request independently of APIC priority

Status: Accepted, 2026-10-09. Amends D-372; closes PW-583.

**Context.** The first complete post-D-373 gate reached release QEMU after all
host, API, documentation and supply-chain stages passed, then failed only `a
pending local interrupt refuses the first sti` at 276/277. An unchanged focused
rerun passed 277/277. The production door was correctly reading all eight IRR
banks and refusing the highest pending vector. The machine witness, however,
declared its queued self-IPI present only when that vector was itself highest
and required the refusal to name it. A legitimate timer request at a higher
vector could therefore hide the self-IPI from the test without bypassing the
door.

**Decision.** One shared helper snapshots all eight IRR banks. Production still
selects and reports their highest pending vector before `sti`. The self-test
separately asks whether the exact self-IPI bit is present in that snapshot and
accepts refusal of any pending vector. It still proves the sent request entered
IRR while interrupts were excluded, the door kept them excluded, no delivery
occurred before unwind, and the self-IPI arrived afterward.

**Proof required.** Pure host logic must find a named pending vector beneath a
higher-priority request and reject an absent neighbour. Mutation 1,759 restores
the erroneous highest-only interpretation and must fail only that host test.
The release machine must still pass all 277 checks with the real self-IPI and
production audit. The final integrated gate must repeat both release and debug
machine witnesses.

**Evidence.** The focused host contract and `x86-apic` suite pass, mutation
1,759 is registered against the exact named-vector test, and the restored
release machine passed 277/277. Final integrated-gate evidence is recorded by
the closing commit.

**Alternatives.** Disarming the timer would make the test alter unrelated live
scheduler state. Raising the self-IPI's vector would test an artificial
priority arrangement. Treating any pending bit as proof that the sent self-IPI
queued would permit unrelated work to satisfy the witness. Exact-bit presence
keeps the observation causal without weakening the production refusal.
