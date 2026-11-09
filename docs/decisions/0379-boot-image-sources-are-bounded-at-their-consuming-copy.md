# D-379 Boot-image sources are bounded at their consuming copy

Status: Accepted (2026-10-10), closes PW-588

**Context.** Every public and private image route converges on the ESP
population helper, but that helper copied the fixed `BOOTX64.EFI` and
`limine.conf` checkout entries with `std::fs::read`. The eager read followed a
final-component link and allocated through end of file before FAT capacity
could bound the destination. An operator building a hostile checkout could
therefore consume attacker-selected host memory or embed bytes outside the
reviewed source tree.

**Decision.** Image assembly reads both fixed sources through the existing
D-344 checked reader inside the consuming copy operation. Every ancestor and
the immediate parent must remain a real directory, the source must be one
direct regular child, and the opened handle cannot follow a final-component
link. Initial length is checked before allocation, one byte past the ceiling
detects growth, and final length plus direct-file shape are rechecked. The
Limine configuration has a 64 KiB ceiling; the EFI executable uses the same
64 MiB reviewed vendored-payload ceiling as the vendor verifier. All image,
run, boot-check, degraded and VMware paths inherit the rule through their one
shared ESP population boundary rather than an optional earlier gate.

**Proof required.** A focused host witness must show that the old copy accepts
an external sentinel through a fixed source link, while the repaired copy
refuses that link and a direct file one byte beyond its ceiling before creating
the destination entry. A bounded direct file must be copied byte for byte. One
retained mutation must restore the eager following read and fail the redirected
source witness; the ordinary boot-image integration tests and complete host
gate must stay green.

**Evidence.** Against the old helper,
`redirected_image_sources_are_refused_before_copy` failed because the external
sentinel was copied successfully. The repaired three-test suite refuses the
link and the 65,537-byte configuration and reads back the accepted direct bytes
exactly. Mutation 1,769 restores `std::fs::read` and fails the exact redirected
witness; restoration returns the suite to green. Final image, host and
integrated-gate evidence is recorded by the closing commit.

**Alternatives.** Relying on `verify-vendor` would leave direct image commands
outside the control and would not cover `limine.conf`. Checking metadata before
the old read retains the link-swap window. Streaming directly into FAT would
avoid one bounded buffer but would need rollback for a source that grows or
changes during the write; the shared checked reader already gives the required
stable, no-follow snapshot with limits far below the image extent.
