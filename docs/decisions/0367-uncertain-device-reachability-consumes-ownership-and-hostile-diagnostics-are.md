# D-367 Uncertain device reachability consumes ownership, and hostile diagnostics are bounded

Status: Accepted, 2026-10-09.

**Decision.** The public VT-d mapping boundary takes a mutable `DmaSegment`,
not a raw address. A clean preflight refusal leaves ownership with the caller.
Once a translation may have been installed, every unsuccessful map, rollback,
unmap or invalidation retains the segment inside its ownership type before
returning failure. Raw address mapping remains private and is used only while
establishing firmware-reserved regions before translation is enabled. Queue
rollback must inspect both segments through this same boundary; transfer
cleanup must never reconstruct ownership policy from an address tuple.

Both QEMU serial consumers enforce one reviewed per-line ceiling and one
aggregate ceiling while bytes arrive. Controlled boots use a private,
backpressured OS pipe or socket rather than a file, so output beyond the
reader's aggregate refusal can occupy only the bounded transport buffer until
the parent terminates QEMU. Parsing sees the bounded, lossy UTF-8 view it
already expects; console display escapes every non-printing byte, including
terminal controls, without rewriting the evidence supplied to the parser.
Unknown-call accounting remains cumulative, but its deferred warning is
one-shot for the boot. API-audit Cargo metadata and compiler runs use the
existing whole-process-tree deadline and combined-output ceiling, a finite 64
MiB per child, because a real host compiler round exceeded the provisional 16
MiB. Kernel ELF reads use one shared maximum in both verification and image
assembly, and the parser separately caps section count and aggregate decoded
name bytes.

The Deep qualification consumer accepts only producer `0.1.32`; fixtures and
negative tests keep stale provider output closed. DMAR validation also bounds
the number of remapping units before its allocation-free duplicate scan, and
equal-specificity units fail closed rather than letting table order choose a
device's isolation authority.

**Rationale.** Codex Security Standard scan
`1b6daa89-e4c0-4bff-8127-fe07bdb8e481` of revision
`8d9f1b966c2223156c109b19acf7b74d028f63c8` found one high, one medium and four
low findings. VT-d mapping returned one boolean for both clean refusal and
uncertain device reachability, so queue setup and transfer cleanup could drop a
DMA segment whose translation or cached reachability had not been conclusively
removed. QEMU runners accepted an unbounded guest serial line and transcript
and printed guest control bytes to the host terminal, repeated unknown calls
could re-arm UART warnings, the API audit used unbounded child capture, and the
host ELF reader admitted an unbounded file and section-name amplification. The
qualified producer had also advanced from `0.1.31` to `0.1.32`.

**Alternatives.** A richer public VT-d result enum still leaves every caller
responsible for remembering quarantine. Silently truncating serial or child
output could manufacture a valid-looking verdict. Escaping before parsing
would change evidence semantics. Trusting ELF shape because rustc emitted it
would make a repository-derived artifact an unbounded host input. Preserving
first-listed DMAR precedence would let ambiguous firmware choose security
authority by ordering alone.

**Consequences.** Machine tests inject post-install map and unmap VT-d
invalidation failures and show affected frames never return to the allocator,
and a partially mapped queue pair rolls its first queue back. Mutations 1,625
through 1,644, with the rebound transport mutations 1,519 and 1,520, each fail
their exact host or machine witness; mutation 1,642 fails only
`uncertain vtd unmapping never recycles its frames`. A complete Deep scan of a
clean revision was left as separate qualification.
