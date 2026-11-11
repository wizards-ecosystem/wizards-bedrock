# D-162 The last unanchored scancode was a misread column

Status: Accepted, 2026-08-25.

**Decision.** The backslash code D-156 left unanchored is settled at `0x5D` and
anchored; there was no conflict. Brouwer's correspondence table gives the key
one code per set: **`0x2B` in set 1, `0x5D` in set 2, `0x5C` in set 3.** Vetra's
translation table agrees independently: set 2 `5D/F0 5D`, set 3
`5C/F0 5C or 53/F0 53`. The `0x5C` was the set 3 value read from the wrong
column. This kernel puts the keyboard in set 2 and never in set 3.

**Rationale.** The source and the table were both correct and the reading was
wrong, which "check your sources" does not catch, because the source was
checked. A table with three columns answers a question that names its column and
misleads one that does not. D-156's refusal to guess was right and is what made
this cheap: an asserted `0x5C` would have been a green test enshrining the set 3
value, failing as one key on real hardware, the class D-156 existed to remove.

**Consequences.**

- `libs/ps2-keys`' set 2 table is anchored at **69 codes**, and nothing in it is
  unanchored on purpose. Given the set 3 code, the test goes red naming the code
  and the key.
- The test's doc comment cites two independent sources and names the set the
  value belongs to, the detail whose absence caused this.
- D-156's "left owed" item is closed.
