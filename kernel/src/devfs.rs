//! Names that yield devices, and **not** bytes.
//!
//! The third backend, and the one that exists to make a point `012` §7 is
//! emphatic about: **a devfs entry has no bytes.** It is a name that yields a
//! handle *of another type*. Forcing it to implement `read_at` is the
//! "everything is a file" conflation, and WASI's own README rejects it on
//! exactly these grounds.
//!
//! So `lookup` on devfs returns a [`Node::Device`](crate::vfs::Node::Device),
//! and `vfs::read` takes a [`File`](crate::vfs::File). **There is no way to
//! pass a device to it** — not a refusal at runtime, an absence at compile
//! time. That is the difference between a design that says devices are not
//! files and one that enforces it (D-172).
//!
//! What is here is a list, not a registry. There is one driver, it is claimed
//! at boot, and a registry with one entry is a description of that entry
//! (D-41). Slice 4's `Device` capability is what a program will eventually
//! receive here; today the name resolves and the handle table entry it becomes
//! is Phase 7's.

/// A device this kernel can name.
///
/// An enum rather than an index into a registry, so a fourth device is a
/// compile error everywhere that matches on it rather than a number that means
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Device {
    /// The storage controller slice 4 claimed.
    Storage,
    /// The console the kernel writes its own log to.
    Console,
}

impl Device {
    /// What it is called in the directory.
    pub(crate) const fn name(self) -> &'static [u8] {
        match self {
            Self::Storage => b"storage",
            Self::Console => b"console",
        }
    }

    /// What kind of thing it is, for a listing that wants to say.
    pub(crate) const fn describes(self) -> &'static str {
        match self {
            Self::Storage => "a block device",
            Self::Console => "a character device",
        }
    }
}

/// Every device, in listing order.
///
/// Exhaustive by construction: the compiler checks the match in
/// [`Device::name`], and this array is checked against it by a test that
/// counts (D-161).
pub(crate) const DEVICES: [Device; 2] = [Device::Storage, Device::Console];

/// Find `name` among the devices.
pub(crate) fn lookup(name: &[u8]) -> Option<Device> {
    DEVICES.into_iter().find(|device| device.name() == name)
}

/// Call `found` for every device.
pub(crate) fn walk(mut found: impl FnMut(&[u8], bool) -> bool) {
    for device in DEVICES {
        // **Never a directory**, and never a file either. The boolean here is
        // "is it a directory", which every backend answers, and a device is
        // neither — which is why the *node* it resolves to is a third thing
        // rather than a flag on a file.
        if !found(device.name(), false) {
            return;
        }
    }
}
