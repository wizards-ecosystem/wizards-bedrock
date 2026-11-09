# D-367 Uncertain device reachability consumes ownership, and hostile diagnostics are bounded

Status: Accepted (2026-10-09)

**Context.** A complete Codex Security Standard scan of revision
`8d9f1b966c2223156c109b19acf7b74d028f63c8` found one high, one medium and
four low findings. VT-d mapping returned one boolean for both clean refusal
and uncertain device reachability. Queue setup and transfer cleanup could
therefore drop a DMA segment whose translation or cached reachability had not
been conclusively removed. QEMU runners accepted an unbounded guest serial
line and aggregate transcript and printed guest control bytes to the host
terminal. Repeated unknown calls could continually re-arm UART warnings. The
API audit used unbounded child capture, and the host ELF reader admitted an
unbounded file and section-name amplification. The qualified Codex Security
producer also advanced from `0.1.31` to `0.1.32`.

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
existing whole-process-tree deadline and combined-output ceiling. Kernel ELF
reads use one shared maximum in both verification and image assembly, and the
parser separately caps section count and aggregate decoded name bytes.

The Deep qualification consumer accepts only producer `0.1.32`; fixtures and
negative tests keep stale provider output closed. DMAR validation also bounds
the number of remapping units before its allocation-free duplicate scan, and
equal-specificity units fail closed rather than letting table order choose a
device's isolation authority.

**Proof required.** Host tests must force every new resource and ambiguity
limit, prove control-byte escaping without altering parser input, prove the
unknown-call warning cannot re-arm, and reject stale Deep output. Machine
tests must inject post-install map and unmap VT-d invalidation failures and
demonstrate that affected frames never return to the allocator. Queue-pair
partial mapping must prove it rolls its first queue back. Each new check needs
a retained red mutation and exact restoration. Focused tests, release and
debug boots, the ordinary gate, and a new complete Deep scan are required
before closure.

**Evidence.** On 2026-10-09 Codex Security Standard scan
`1b6daa89-e4c0-4bff-8127-fe07bdb8e481` completed against revision
`8d9f1b966c2223156c109b19acf7b74d028f63c8`; the implementation and a single
post-patch review resolved all six findings and added independent unmap,
partial-queue-rollback and repeated-name amplification witnesses. Mutations
1,625 through 1,644, together with the rebound transport mutations 1,519 and
1,520, each failed its exact host or machine witness before restoration. The
restored release mutation boot passed 273/273 after mutation 1,642 failed only
`uncertain vtd unmapping never recycles its frames`.

The first full gate correctly refused the API audit when a real host compiler
round exceeded its provisional 16 MiB combined-output ceiling. A finite 64
MiB per-child ceiling then admitted the reviewed workload while retaining live
polling, collection and process-tree bounds. The restored `cargo xtask ci`
passed: all 1,908 public API items had compiler-resolved callers, the `xtask`
suite passed 719 tests with ten deliberate ignores, both release and debug
self-test boots passed 273/273, and shipped, degraded and no-IOMMU boots all
settled with their exact required outcomes. The complete Deep scan remains the
separate clean-revision qualification required below.

**Alternatives.** A richer public VT-d result enum still leaves every caller
responsible for remembering quarantine. Silently truncating serial or child
output could manufacture a valid-looking verdict. Escaping before parsing
would change evidence semantics. Trusting ELF shape because rustc emitted it
would make a repository-derived artifact an unbounded host input. Preserving
first-listed DMAR precedence would let ambiguous firmware choose security
authority by ordering alone.
