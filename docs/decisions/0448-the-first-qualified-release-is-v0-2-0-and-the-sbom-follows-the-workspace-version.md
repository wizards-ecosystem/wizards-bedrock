# D-448 The first qualified release is v0.2.0 and the SBOM follows the workspace version

Status: Accepted, 2026-10-27. Applies D-425 and D-441.

**Decision.** The workspace version becomes 0.2.0, and the first qualified
diagnostic release is tagged `v0.2.0` on the commit that passes D-425's gate.
The SBOM writer takes the project package version from the workspace version
it is compiled with, so the regenerated document and `cargo xtask
supply-chain` bind the released number without a second edit.

**Rationale.** D-441 keeps v0.1.0 an immutable source preview and requires a
later qualified release to take another version, and D-425 tags the qualified
commit directly, so that commit must already carry the version it releases.
The SPDX project package version had been a literal `0.1.0` in `xtask` that
would have described the release as the preview and needed an edit nothing
checks. A patch version would understate the change from an unqualified preview
to a qualified release.
