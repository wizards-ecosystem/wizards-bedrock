#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Device {
    Storage,
    Console,
}

impl Device {
    pub(crate) const fn name(self) -> &'static [u8] {
        match self {
            Self::Storage => b"storage",
            Self::Console => b"console",
        }
    }

    pub(crate) const fn describes(self) -> &'static str {
        match self {
            Self::Storage => "a block device",
            Self::Console => "a character device",
        }
    }
}

pub(crate) const DEVICES: [Device; 2] = [Device::Storage, Device::Console];

pub(crate) fn lookup(name: &[u8]) -> Option<Device> {
    DEVICES.into_iter().find(|device| device.name() == name)
}

pub(crate) fn walk(mut found: impl FnMut(&[u8], bool) -> bool) {
    for device in DEVICES {
        if !found(device.name(), false) {
            return;
        }
    }
}
