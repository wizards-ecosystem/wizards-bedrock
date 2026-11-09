# D-380 ESP population authenticates the exact bootloader buffer

Status: Accepted, 2026-10-10. Closes PW-589.

**Context.** `boot/limine/CHECKSUMS` pins the reviewed SHA-256 of
`BOOTX64.EFI`, and the vendor gate verifies it, but image, run, boot and VMware
commands could be invoked directly. Their shared ESP population path embedded
the bounded checkout bytes without consulting the pin. Replacing only the EFI
payload therefore gave substituted pre-kernel code control of an image even
though the repository carried a different expected identity.

**Decision.** The one ESP consumer now reads `BOOTX64.EFI` once through
D-379's bounded direct-file boundary, reads the adjacent `CHECKSUMS` through
the existing bounded no-follow listing reader, and requires that listing to
contain exactly one entry naming the consumed payload. It hashes the already
read buffer, compares it to that entry, and writes that same buffer to the FAT
file only after the comparison succeeds. Thus every public or private image
route authenticates at consumption; no optional dispatch-stage verifier and no
second payload open sits between authentication and embedding.

**Proof required.** With a checksum for reviewed bytes left unchanged, a
fixed-path substituted bootloader must make image population fail before the
destination EFI file exists. Matching bytes must populate successfully and
read back exactly. The checksum consumer must refuse a redirected listing,
multiple entries, a name other than the consumed adjacent payload, and a
payload path outside the listing's directory. A retained mutation must remove
only the consumption-time comparison and fail the substituted-payload witness
while D-379's independent path and size mutation remains live.

**Evidence.** Against the old ESP consumer, the substituted-payload witness
failed because population returned success and embedded the unpinned bytes.
The repaired five-test ESP suite refuses that payload before creation, embeds
matching bytes exactly, and retains the redirect, oversize and direct-byte
checks. The focused vendor witnesses accept one exact adjacent entry and
refuse a redirected listing plus the multiple, wrong-name and foreign-parent
cases. Mutation 1,770 removes the one authentication call and fails exactly
`image_population_refuses_a_bootloader_that_misses_its_pinned_digest`; mutation
1,769 independently restores the unbounded following source read. Final host,
image and integrated-gate evidence is recorded by the closing commit.

**Alternatives.** Calling `verify-vendor` from each task arm would duplicate
policy, miss future image entrypoints and reopen the payload before copying.
Verifying once before dispatch would retain the same substitution window.
Compiling a second digest constant into `xtask` would create two pins that can
drift; the direct checked listing remains the single reviewable source record,
while the consumer binds it to the exact bytes it embeds.
