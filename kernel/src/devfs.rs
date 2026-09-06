//! Names that yield devices rather than byte-bearing files.
//!
//! Lookup returns [`Node::Device`](crate::vfs::Node::Device), while reads take
//! [`File`](crate::vfs::File), enforcing the distinction in types (D-172).

/// A device this kernel can name.
///
/// An enum rather than an index into a registry, so a new device is a
/// compile error everywhere that matches on it rather than a number that means
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Device {
    /// Block storage owned by the claimed controller.
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
/// The host source contract inventories this independently from the enum and
/// both descriptive matches (D-174).
pub(crate) const DEVICES: [Device; 2] = [Device::Storage, Device::Console];

/// Find `name` among the devices.
pub(crate) fn lookup(name: &[u8]) -> Option<Device> {
    DEVICES.into_iter().find(|device| device.name() == name)
}

/// Call `found` for every device.
pub(crate) fn walk(mut found: impl FnMut(&[u8], bool) -> bool) {
    for device in DEVICES {
        if !found(device.name(), false) {
            return;
        }
    }
}
