# D-83 Two rules that cost nothing and keep three options open

Status: Accepted, 2026-08-24.

**Context.** `010` §10 surveyed the 2026 hardware security features against what
Phase 5 is about to freeze. Most need nothing. Ordinary code written without
them in mind forecloses three: PKS, LAM and its successor ChkTag. Keeping them
open costs two rules.

**Decision.**

1. **Reserve PTE bits 62:59.** Build nothing on them.
2. **Range-check user pointers. Never test `>> 47 == 0`.**

**Rationale.** PKS is Sapphire Rapids and Intel-only and was **never merged**
into Linux, so building on it would be speculative. Its bits are in the PTE,
though, and a page-table encoding that spends them is a format change later.
Reserving four bits costs no design.

The pointer rule guards against another breakage. `>> 47 == 0` is the common
idiom and is correct today. It is wrong under LAM, 5-level paging and ChkTag,
which all put non-address bits in the top of a pointer. LAM itself is being
superseded by ChkTag and is not worth building for. The rule holds anyway: a
range check is as short as the shift and right in every case. `010` §10's own
phrasing: range-check user pointers, never `>> 47 == 0`.

**Alternatives.** Build PKS support (rejected: never merged upstream, Intel
only, no caller). Say nothing and let the idiom appear (rejected: it is the
default thing to write, once per user-pointer path).

**Consequences.**

- `libs/x86-paging` documents bits 62:59 as reserved.
- Slice 3's user-pointer validation is a range check, and slice 5's
  `copy_from_user`/`copy_to_user` call it before taking the SMAP guard.
- LASS needs only discipline, which the range check already gives: the split is
  exactly bit 63, and `#GP` becomes the null-deref reporter.
