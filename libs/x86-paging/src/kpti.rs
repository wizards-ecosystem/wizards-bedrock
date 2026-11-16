use core::fmt;

use crate::{Entry, Flags, MAX_PHYSICAL, PAGE_SIZE, page_identity_matches};

const HIGHER_HALF_START: u64 = 0xffff_8000_0000_0000;

pub const fn shared_user_link_matches(full: Entry, stripped: Entry) -> bool {
    if !full.is_present() || !stripped.is_present() {
        return full.bits() == Entry::ABSENT.bits() && stripped.bits() == Entry::ABSENT.bits();
    }

    let permitted = Flags::PRESENT
        .union(Flags::WRITABLE)
        .union(Flags::USER)
        .union(Flags::ACCESSED);
    full.table_identity_matches(stripped)
        && full.flags().difference(permitted).bits() == 0
        && full.flags().contains(Flags::PRESENT)
        && full.flags().contains(Flags::WRITABLE)
        && full.flags().contains(Flags::USER)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnclavePermission {
    Executable,
    ReadOnly,
    Writable,
}

impl EnclavePermission {
    pub const fn flags(self) -> Flags {
        match self {
            Self::Executable => Flags::PRESENT,
            Self::ReadOnly => Flags::PRESENT.union(Flags::NO_EXECUTE),
            Self::Writable => Flags::PRESENT
                .union(Flags::WRITABLE)
                .union(Flags::NO_EXECUTE),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EnclavePage {
    virtual_address: u64,
    physical_address: u64,
    permission: EnclavePermission,
}

impl EnclavePage {
    pub const fn new(
        virtual_address: u64,
        physical_address: u64,
        permission: EnclavePermission,
    ) -> Result<Self, EnclavePageError> {
        if !virtual_address.is_multiple_of(PAGE_SIZE) {
            return Err(EnclavePageError::MisalignedVirtual(virtual_address));
        }
        if virtual_address < HIGHER_HALF_START {
            return Err(EnclavePageError::OutsideHigherHalf(virtual_address));
        }
        if !physical_address.is_multiple_of(PAGE_SIZE) {
            return Err(EnclavePageError::MisalignedPhysical(physical_address));
        }
        if physical_address >= MAX_PHYSICAL {
            return Err(EnclavePageError::PhysicalOutOfRange(physical_address));
        }
        Ok(Self {
            virtual_address,
            physical_address,
            permission,
        })
    }

    pub const fn virtual_address(self) -> u64 {
        self.virtual_address
    }

    pub const fn physical_address(self) -> u64 {
        self.physical_address
    }

    pub const fn permission(self) -> EnclavePermission {
        self.permission
    }

    fn matches(self, observed: ObservedPage) -> bool {
        self.virtual_address == observed.virtual_address
            && page_identity_matches(
                observed.physical_address,
                observed.flags,
                observed.span,
                self.physical_address,
                self.permission.flags(),
            )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnclavePageError {
    MisalignedVirtual(u64),
    OutsideHigherHalf(u64),
    MisalignedPhysical(u64),
    PhysicalOutOfRange(u64),
}

impl fmt::Display for EnclavePageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MisalignedVirtual(address) => {
                write!(
                    formatter,
                    "enclave address {address:#x} is not page-aligned"
                )
            }
            Self::OutsideHigherHalf(address) => {
                write!(
                    formatter,
                    "enclave address {address:#x} is not in the higher half"
                )
            }
            Self::MisalignedPhysical(address) => {
                write!(
                    formatter,
                    "enclave backing {address:#x} is not page-aligned"
                )
            }
            Self::PhysicalOutOfRange(address) => {
                write!(
                    formatter,
                    "enclave backing {address:#x} does not fit 52 bits"
                )
            }
        }
    }
}

impl core::error::Error for EnclavePageError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ObservedPage {
    virtual_address: u64,
    physical_address: u64,
    span: u64,
    flags: Flags,
}

impl ObservedPage {
    pub const fn new(virtual_address: u64, physical_address: u64, span: u64, flags: Flags) -> Self {
        Self {
            virtual_address,
            physical_address,
            span,
            flags,
        }
    }
}

// `observed` must hold every present stripped higher-half leaf: an omitted one reads as absent.
pub fn enclave_matches(expected: &[EnclavePage], observed: &[ObservedPage]) -> bool {
    if expected.is_empty() || expected.len() != observed.len() {
        return false;
    }

    for (index, page) in expected.iter().enumerate() {
        if expected[..index].iter().any(|prior| {
            prior.virtual_address == page.virtual_address
                || prior.physical_address == page.physical_address
        }) {
            return false;
        }
        let mut match_count = 0;
        for candidate in observed {
            if candidate.virtual_address == page.virtual_address {
                match_count += 1;
                if !page.matches(*candidate) {
                    return false;
                }
            }
        }
        if match_count != 1 {
            return false;
        }
    }
    true
}
