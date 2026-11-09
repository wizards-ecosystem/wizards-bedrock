# D-273 An empty repeated MMIO access touches nothing

Status: Accepted, 2026-08-31. Amends D-105 and D-136; advances PW-522.

**Context.** `AccessRange` claimed zero width was impossible while its own
constructor admitted a zero count, then suppressed the lint which exposed that
contradiction. The first exhaustive oracle could also accept arbitrary false
refusals, so it did not decide what an empty request meant.

**Decision.** A repeated access still requires a valid power-of-two element
width. With count zero it is an empty no-op at any offset at or before the
region end; alignment is irrelevant because no address is touched. An offset
past the region remains out of bounds. `AccessRange` exposes the exact checked
offset iterator rather than a misleading byte-length API, and the empty
iterator yields no pointer candidate.

**Alternatives and current evidence.** Refusing every empty span was rejected
because callers can naturally produce zero work and no device precondition is
needed for an access which does not occur. Admitting an offset outside the
region was rejected because it would make the range cease to describe that
region even though iteration is empty. An independent bounded oracle compares
every acceptance and exact error class across widths, counts, offsets and
region lengths. On 2026-08-31 the exhaustive width/alignment/range domain and
current MMIO consumers passed; empty-span, repeated-offset and refusal-class
mutations each failed their exact host witness before restoration, closing
PW-522.
