//! Device names: lookup returns `Node::Device` while reads take `File`, so types keep them apart.

/// A device this kernel can name, an enum so a new device is a compile error at every match.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// Every device in listing order; a host contract checks it against the enum and both matches.
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
