//! First-light banner.

/// Shown on the framebuffer console once it is up.
pub(crate) const LINES: &[&str] = &[
    "",
    "  The Wizard's Bedrock",
    "  ~~~~~~~~~~~~~~~~~~~~",
    "",
    "  first light: hello from ring 0",
    "  boot: Limine / UEFI / x86_64 higher half",
];
