use x86_paging::{CanonicalAddress, Entry, Flags, Level};

use super::super::{cpu, interrupt};
use super::{Flush, active_root, is_active, leaf_in};
use crate::mm::{self, PhysicalAddress};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DirectAccess {
    Writable,
    ReadOnly,
}

// Rewrites a managed frame's 4 KiB direct-map leaf, the one present entry that may change in place.
pub(crate) fn set_direct_access(physical: PhysicalAddress, access: DirectAccess) -> Flush {
    assert!(
        is_active(),
        "a direct-map leaf changed before our tables were live"
    );
    let Some(address) = mm::direct_map()
        .and_then(|direct| direct.virtual_address(physical.get()))
        .and_then(CanonicalAddress::new)
    else {
        panic!("managed frame {physical:?} has no direct-map address");
    };
    let slot = leaf_in(active_root(), address.get())
        .unwrap_or_else(|reason| panic!("managed frame {physical:?} has no 4 KiB leaf: {reason}"));
    assert!(
        slot.read().address() == physical.get(),
        "a direct-map leaf names another frame"
    );
    let flags = match access {
        DirectAccess::Writable => Flags::PRESENT
            .union(Flags::WRITABLE)
            .union(Flags::NO_EXECUTE),
        DirectAccess::ReadOnly => Flags::PRESENT.union(Flags::NO_EXECUTE),
    };
    let entry = Entry::page(Level::Pt, physical.get(), flags)
        .unwrap_or_else(|reason| panic!("a frame address failed to encode: {reason:?}"));
    let disabled = interrupt::disable();
    slot.write(entry);
    cpu::invalidate_page(address);
    disabled.restore();
    Flush::none()
}
