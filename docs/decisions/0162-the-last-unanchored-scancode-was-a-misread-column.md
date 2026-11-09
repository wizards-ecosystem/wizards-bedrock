# D-162 The last unanchored scancode was a misread column

Status: Accepted, 2026-08-25.

**Context.** D-156 transcribed 68 of about 89 single-byte set 2 make codes and
stopped one short. Backslash read as `0x5C` in the source and the table had
`0x5D`; mainstream references sided with the table, a second primary source
could not be reached that day, and the entry was left out with the reason
written down — *"one code unanchored and said so beats sixty-nine with one
invented."*

**Decision.** It is settled and anchored. There was no conflict.

Brouwer's correspondence table gives the backslash key three codes, one per
set: **`0x2B` in set 1, `0x5D` in set 2, `0x5C` in set 3.** Vetra's
translation table says the same independently — set 2 `5D/F0 5D`, set 3
`5C/F0 5C or 53/F0 53`. The `0x5C` that looked like a contradiction is the
**set 3** value, read out of the wrong column of a table that carries all
three.

This kernel puts the keyboard in set 2 and never in set 3, so `0x5D` is the
code it will see. The table's value was right the whole time; what was missing
was the reason to believe it.

**Rationale.** Worth an entry of its own for what the shape of the mistake
teaches. The source was correct, the table was correct, and the reading was
wrong — which is not a case that "check your sources" catches, because the
source *was* checked. What caught it was going back with a specific question:
not "what is backslash" but "what is backslash **in set 2**, and what is at
`0x5C` if not this". A table with three columns answers a question that names
its column and misleads one that does not.

D-156's refusal to guess was right and is what made this cheap. An asserted
`0x5C` would have been a green test enshrining the set-3 value, and the failure
would have shown up as one key on real hardware — which is exactly the class
D-156 existed to remove.

**Consequences.**

- `libs/ps2-keys`' set 2 table is anchored at **69 codes**, up from 68, and
  nothing in it is now unanchored on purpose. Broken once: the table given the
  set-3 code goes red naming the code and the key.
- Two independent sources are cited in the test's doc comment rather than one,
  and the set the value belongs to is named — which is the detail whose absence
  caused this.
- D-156's "left owed" item is closed.
