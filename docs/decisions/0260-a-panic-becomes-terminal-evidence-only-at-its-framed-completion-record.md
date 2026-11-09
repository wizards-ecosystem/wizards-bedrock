# D-260 A panic becomes terminal evidence only at its framed completion record

Status: Accepted (2026-08-31), amends D-209 and D-256; advances PW-029

**Context.** The boot readers published their panic flag as soon as a line
contained `KERNEL PANIC`. The watcher could then stop QEMU before later lines
from the same `PanicInfo` reached the reader. Joining the reader preserved only
bytes already read; it did not prove the exact cause was complete. A terminal
mutation could therefore become inconclusive or be attributed from a truncated
transcript depending on scheduling.

**Decision.** Version 1 is exactly two newline-terminated physical records: one
opening/body record and one completion record. The allocation-free writer
escapes backslash, carriage return and line feed injectively before either can
reach the framing transport. It emits the body separator and completion only
after every prior formatter write succeeds. The UART keeps its bounded polling
but returns `fmt::Error` for any byte it cannot transmit, including the inserted
carriage return, so a truncated body cannot be followed by a completion from
the same call.

The self-test, shipped and degraded readers share one exact newline- and
severity-framed parser. They remove only the transport LF and its optional CR,
preserve every other byte, and reject an unterminated or internally CR/LF
opening, a continuation, a second opening, completion before opening, duplicate
completion and all post-terminal data. Only the distinct completion transition
publishes terminal state. Degraded attribution additionally requires the exact
canonical no-x2APIC warning record composed by the kernel and `wost`; a marker
substring, wrong severity or altered prefix/suffix is not prior diagnosis.

**Alternatives and evidence.** Moving the old atomic store after the opening
line was rejected because it still races a partial write. A fixed delay was
rejected because time is not a framing boundary. Literal and pairwise
round-trip host oracles distinguish escaped line breaks from literal escape
text; a one-shot writer refuses every character position and proves no
completion follows an error. Exact parser fixtures cover whitespace retention,
malformed framing, every state transition and no-x2APIC near misses, while
source contracts bind the kernel adapter, UART error propagation and canonical
diagnosis producers. Manual breaks exposed and repaired two initially
self-derived witnesses: an omitted completion and a post-terminal refusal
credited to the continuation guard. The retained mutations cover each distinct
writer, framing, state, diagnosis, attribution and UART branch. The real
timer-panic and restored degraded boots remain PW-029's machine close.
