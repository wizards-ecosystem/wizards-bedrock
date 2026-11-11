# D-145 Three claims the code had outrun, and one of them lost messages

Status: Accepted, 2026-08-25.

**Decision.** Comments that assert away a real behaviour are defects, and the
code or the comment is corrected:

- **`receive_blocking`'s orphan arm drains rather than refuses**: `return
  self.receive()` makes "the far end closed with nothing left to read" true in
  both orderings. Taking a message and testing orphanhood in two acquisitions,
  with interrupts on between them, let a peer that sent *and then* closed get
  `None` with its message still queued.
- **`await_key`'s drain takes a cap of its own**, two batches per pass, so a
  controller with more to say gets the next pass and one that never stops
  saying it does not get the loop. Its doc's "doubly bounded" was true only of
  the outer loop; it now says three bounds and names what each catches.
- **The input module doc says `Events::push` drops on a full sixteen-deep ring
  and counts it**, not that every event is queued.
- **`# Errors` blocks match their bodies**: `Channel::send` cannot return
  `OutOfMemory`, because the inbox is reserved to `DEPTH` at construction, and
  `elf64::Elf::parse` can return `WrongMachine`.

**What the check establishes, and what it does not.** It pins the **ordering**,
draining before refusing: test orphanhood first and it goes red. It does **not**
reproduce the window, and cannot single-threaded, so removing the fixed line
alone leaves it green. That is written into the check, as D-141's limit is.

**Rationale.** Each comment described the **intent** of the surrounding code
and the code implemented it *almost*; the inner loop was added later by someone
who read the doc and believed it. The reader of a false comment is the next
author, and the comment is what tells them the hazard is handled. Nothing
enforces `# Errors`, and nothing can: it is prose about a `Result`.

**Left owed.** The two-thread window in `receive_blocking` is unreproduced, as
is D-141's arena race. Both need concurrency this kernel does not yet have, and
both would need a flake budget to test: a decision for whoever adds a second
CPU, alongside D-126's `LEAVES_HELD` and D-129's `pic::silence` race, all four
of which are waiting on the same thing.
