# D-211 A boot image names and verifies the kernel it actually carries

Status: Accepted, 2026-08-28. Amends D-135; closes PW-500.

**Context.** D-135 repaired the VMware phase-close image after finding that it
was built with an empty feature slice and therefore ran none of the checks. Its
guard reads one dispatch arm as source text and looks for `SELFTEST_FEATURE`.
That protects yesterday's spelling at one caller, not the boundary: every
image caller can still pass any `&[&str]`, including no feature for a check
image or both test-only features together. The builder also trusts the path it
copied. Nothing reopens the FAT image to establish which kernel bytes will
reach Limine.

**Decision.** `image::Kernel::{Shipped, SelfTest}` is the complete selector for
normal boot images. Its exhaustive private mapping is the only place where a
kernel identity becomes Cargo features; no raw feature slice crosses the image
module boundary. The fault-injected PCI witness has a separately named
`build_degraded` constructor which fixes the identity to `Shipped` and adds
only `refuse-pci`, so a self-test/degraded combination is not expressible.

After Cargo and the ELF verifier produce the source artifact, the image builder
reads it once, writes those bytes into the ESP, reopens `/boot/wizard-kernel.elf`
through `fatfs`, and requires byte equality. It then identifies the bytes by
the `selftest: verdict ` signature already consumed by the QEMU harness. That
literal is compiled only with the kernel's `selftest` module; presence means
`SelfTest`, while absence means `Shipped` only after the bytes have matched the
verified ELF. The requested and observed identities must agree before an image
path is returned.

**Rationale.** An enum removes the ambiguous empty slice and the invalid
feature combinations from every caller. Reading the filesystem payload checks
the artifact the firmware will consume rather than the host path the builder
intended to copy. The verdict signature is behavior the self-test kernel must
retain for the existing harness, so it is stronger evidence than a sidecar
label written from the same selector.

**Alternatives.** Keeping the source-text VMware check was rejected because a
rename can satisfy it without constraining the API, and it says nothing about
the other callers. Separate `build_shipped` and `build_selftest` functions were
rejected because the enum makes selection explicit while keeping one assembly
path. A kind file in the ESP was rejected because the builder and its label
could agree while the compiled kernel disagreed. Distinct phantom-typed image
handles were rejected for now because QEMU and VMware only consume a path; the
defect is at construction, where an exhaustive selector and artifact read-back
close it directly.

**Consequences.** The former VMware source-text test is replaced by direct
selector and raw-image round-trip checks. Both kinds were written into test
images, read back through their partition and FAT path, compared byte for byte,
and identified. Mapping `SelfTest` to no features made
`selectors_build_only_their_named_kernel` red; opening `hello.elf` in place of
the kernel made `both_kernel_kinds_are_read_back_and_identified` red. Both
transformations compiled, made their exact host witness red and were restored.
Persistent mutations retain them, bringing the registry to 188.

The real release shipped image then reported `shipped kernel read back`; the
release self-test image reported `self-test kernel read back` before QEMU ran
all 211 checks to a passing verdict. The same post-write verification is on
every normal, VMware, gate and degraded image construction path.
