# D-83 Two rules that cost nothing and keep three options open

Status: Accepted, 2026-08-24.

**Decision.** Two rules keep PKS, LAM and its successor ChkTag open:

1. **Reserve PTE bits 62:59.** Build nothing on them.
2. **Range-check user pointers. Never test `>> 47 == 0`.**

**Rationale.** PKS was never merged into Linux, so building on it would be
speculative, but its bits are in the PTE and spending them is a format change
later. `>> 47 == 0` is correct today and wrong under LAM, 5-level paging and
ChkTag, which all put non-address bits in the top of a pointer; a range check
is as short as the shift and right in every case.

**Alternatives.** Build PKS support (rejected: never merged upstream, Intel
only, no caller). Say nothing and let the idiom appear (rejected: it is the
default thing to write, once per user-pointer path).

**Consequences.**

- `libs/x86-paging` documents bits 62:59 as reserved.
- Slice 3's user-pointer validation is a range check, and slice 5's
  `copy_from_user`/`copy_to_user` call it before taking the SMAP guard.
- LASS needs only discipline, which the range check already gives: the split is
  exactly bit 63, and `#GP` becomes the null-deref reporter.
