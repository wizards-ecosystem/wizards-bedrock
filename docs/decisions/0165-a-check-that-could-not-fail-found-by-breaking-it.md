# D-165 A check that could not fail, found by breaking it

Status: Accepted, 2026-08-25.

**Decision.** The check for D-151's property, **a promised handle slot is given
back on every terminal path**, is rebuilt so that every submission reserves and
then fails, at a different point each time: at the lookup (a value naming
nothing of this table's), at the right (a handle held without `DUPLICATE`), and
at the attenuation (asking for more than is held). The table is deliberately
roomy. The first version used a full table, so every refusal happened at
`reserve_slot`, no promise was ever made, and deleting the destructor left it
green; only D-28's break-once rule found it, when the break reddened a
different check whose table had room.

*A full table refuses at submission* is a different claim and keeps its own
check: it is about the moment the program is told, which D-151 argues is the
whole point of reserving at all. Merging the two is how the first became
unfailable.

**Rationale.** A leak here is a table that shrinks by one every time a program
asks for something it cannot have, surfacing hours later as a process that
cannot open anything. This is D-155's rule arriving from a direction it did not
cover: there the inputs made two answers indistinguishable; here the input made
the code under test unreachable, which no reading of the assertion reveals,
because the assertion is correct.

> **A check must reach the code it is about.** Set-up chosen to make the failure
> easy to trigger can make it impossible to reach — a full table refuses early,
> a closed channel refuses early, an unmapped page faults early. If the refusal
> under test is the *second* thing that could go wrong, the input has to get
> past the first.

`005`'s testing section carries it.

**Consequences.**

- The break that found it is a mutation, so it cannot rot back.
- Two checks assert `promised() == 0` from different set-ups, which is the
  redundancy that caught this and is worth keeping.
