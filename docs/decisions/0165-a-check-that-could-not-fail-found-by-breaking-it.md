# D-165 A check that could not fail, found by breaking it

Status: Accepted (2026-08-25)

**Context.** Slice 1's most important property is D-151's: **a promised handle
slot is given back on every terminal path.** A leak there is not a crash. It is
a table that shrinks by one every time a program asks for something it cannot
have, and it surfaces hours later as a process that cannot open anything.

The check written for it set up a table with room for exactly one handle, put
the source handle in it, and submitted three operations that were refused. It
asserted afterwards that nothing was still promised, and it passed.

**It could not have failed.** With the table already full, all three refusals
happened *at the reservation* — `reserve_slot` returned `QuotaExhausted` and no
promise was ever made. `promised() == 0` was true because nothing had been
promised. Deleting the destructor whose entire job is returning promises left
the check green.

**How it was found.** By D-28, and only by D-28. The break was applied, the
boot went red — and it went red on a *different* check, which is what gave it
away. `a completion cannot widen what was offered` also asserts `promised() ==
0`, and its table has room, so its refusal happens after a promise is taken.
The check that was *about* the property could not see it; a check that
mentioned it in passing could.

**Decision.** The check is rebuilt so that every submission reserves and then
fails, at a different point each time: at the lookup (a value naming nothing of
this table's), at the right (a handle held without `DUPLICATE`), and at the
attenuation (asking for more than is held). The table is deliberately roomy,
which is the opposite of the instinct that produced the first version.

The full-table case is not deleted — it is a different claim and gets its own
check. *A full table refuses at submission* is about the moment the program is
told, which D-151 argues is the whole point of reserving at all. Merging the
two is how the first one came to be unfailable: it was trying to be about both.

**Rationale.** This is D-155's rule — choose inputs where the answers differ —
arriving from a direction that rule did not cover. There the inputs made two
*answers* indistinguishable. Here the input made the code path under test
**unreachable**: the check never ran the lines it was written about. That is
worse, because no amount of staring at the assertion reveals it. The assertion
is correct.

The general form, and it is the third thing this rule has cost:

> **A check must reach the code it is about.** Set-up chosen to make the failure
> easy to trigger can make it impossible to reach — a full table refuses early,
> a closed channel refuses early, an unmapped page faults early. If the refusal
> under test is the *second* thing that could go wrong, the input has to get
> past the first.

`005`'s testing section carries it.

**Consequences.**

- One check rebuilt, one check added, and the break that found it is now a
  mutation, so it cannot rot back.
- Two checks now assert `promised() == 0` from different set-ups, which is the
  redundancy that caught this and is worth keeping.
- Nothing in the kernel changed. The destructor was correct; the check could
  not show it.
