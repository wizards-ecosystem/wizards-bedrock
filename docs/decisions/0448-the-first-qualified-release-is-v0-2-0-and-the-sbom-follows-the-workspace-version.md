# D-448 The first qualified release is v0.2.0 and the SBOM follows the workspace version

Status: Accepted, 2026-10-27. Applies D-425 and D-441.

**Context.** D-441 keeps v0.1.0 an immutable source preview and requires a
later qualified release to take another version. D-425 tags the qualified
commit directly, so that commit must already carry the version it releases.
The SPDX document's project package version was a literal `0.1.0` in
`xtask`, independent of `Cargo.toml`, and would have described the next
release as the preview.

**Decision.** The workspace version becomes 0.2.0, and the first qualified
diagnostic release is tagged `v0.2.0` on the commit that passes D-425's gate.
The SBOM writer takes the project package version from the workspace version
it is compiled with, so the regenerated document and `cargo xtask
supply-chain` bind the released number without a second edit.

**Alternatives.** A patch version would understate the change from an
unqualified preview to a qualified release. Tagging without bumping the
source would publish an archive that names the preview's version. Keeping the
SBOM literal would require a separate edit that nothing checks.
