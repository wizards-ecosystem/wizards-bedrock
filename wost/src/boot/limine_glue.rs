//! The only file in the kernel that names the bootloader protocol.

use limine::BaseRevision;
use limine::framebuffer::MemoryModel;
use limine::memory_map::EntryType;
use limine::request::{
    ExecutableAddressRequest, FramebufferRequest, HhdmRequest, MemoryMapRequest, RsdpRequest,
};

use pixel::{Channel, PixelFormat};

use crate::boot::{
    BootInfo, DirectMap, Framebuffer, FramebufferError, FramebufferInfo, HandoffError,
    ImageAddress, MemoryKind, MemoryMap, MemoryRegion,
};
use crate::mmio::MmioRegion;

/// The boot protocol revision this kernel requests (D-42).
///
/// A function rather than a `const` of the tag type: `BaseRevision` holds
/// `UnsafeCell`s, and a `const` containing one is what
/// `clippy::declare_interior_mutable_const` exists to catch.
///
/// Revision 4 is the first whose direct map reaches a device hole, and the
/// first that guarantees the ACPI tables are mapped — the two things Phase 3
/// needs from the bootloader. `BaseRevision::new()` is revision 3, so asking
/// for anything is a deliberate act; see D-42 for why not 5 or 6.
pub const fn base_revision() -> BaseRevision {
    BaseRevision::with_revision(4)
}

/// Bits per pixel the framebuffer code implements.
const SUPPORTED_BPP: u16 = 32;

/// Bytes per framebuffer access, and therefore the alignment it needs.
const WORD: usize = size_of::<u32>();

/// Assemble [`BootInfo`] from the bootloader's answers and hand over control.
///
/// Called only by [`entry!`](crate::entry).
pub fn enter(
    base_revision: &'static BaseRevision,
    framebuffer: &'static FramebufferRequest,
    memory_map: &'static MemoryMapRequest,
    hhdm: &'static HhdmRequest,
    image: &'static ExecutableAddressRequest,
    rsdp: &'static RsdpRequest,
    kernel_main: fn(BootInfo) -> !,
) -> ! {
    kernel_main(BootInfo {
        base_revision_supported: base_revision.is_supported(),
        framebuffer: take_framebuffer(framebuffer),
        memory_map: take_memory_map(memory_map),
        direct_map: take_direct_map(hhdm),
        image: take_image(image),
        rsdp: take_rsdp(rsdp),
    })
}

/// Where the bootloader placed the kernel image.
fn take_image(request: &'static ExecutableAddressRequest) -> Result<ImageAddress, HandoffError> {
    let response = request.get_response().ok_or(HandoffError::NoResponse)?;
    Ok(ImageAddress::new(
        response.physical_base(),
        response.virtual_base(),
    ))
}

/// The ACPI root pointer's address.
fn take_rsdp(request: &'static RsdpRequest) -> Result<u64, HandoffError> {
    let response = request.get_response().ok_or(HandoffError::NoResponse)?;
    match response.address() as u64 {
        0 => Err(HandoffError::Empty),
        address => Ok(address),
    }
}

/// The bootloader's memory map, translated into types that name no bootloader.
fn take_memory_map(request: &'static MemoryMapRequest) -> Result<MemoryMap, HandoffError> {
    let response = request.get_response().ok_or(HandoffError::NoResponse)?;
    let entries = response.entries();
    if entries.is_empty() {
        return Err(HandoffError::Empty);
    }

    let mut map = MemoryMap::new();
    for entry in entries {
        map.push(MemoryRegion {
            base: entry.base,
            len: entry.length,
            kind: kind_of(entry.entry_type),
        });
    }
    Ok(map)
}

/// Translate one protocol entry type.
///
/// The known types are compared against the crate's own constants rather than
/// against numbers, so a value moving underneath us is a compile error. The raw
/// value is read only to *report* a type we do not know: base revision 4 adds
/// one (`RESERVED_MAPPED`) that the pinned 0.5.0 crate has no constant for, and
/// D-42 requires that such a type be nameable in the log rather than folded
/// silently into `Reserved`.
fn kind_of(entry_type: EntryType) -> MemoryKind {
    match entry_type {
        EntryType::USABLE => MemoryKind::Usable,
        EntryType::BOOTLOADER_RECLAIMABLE => MemoryKind::BootloaderReclaimable,
        EntryType::EXECUTABLE_AND_MODULES => MemoryKind::KernelAndModules,
        EntryType::FRAMEBUFFER => MemoryKind::Framebuffer,
        EntryType::ACPI_RECLAIMABLE => MemoryKind::AcpiReclaimable,
        EntryType::ACPI_NVS => MemoryKind::AcpiNvs,
        EntryType::BAD_MEMORY => MemoryKind::BadMemory,
        EntryType::RESERVED => MemoryKind::Reserved,
        // SAFETY: `EntryType` is `#[repr(transparent)]` over `u64`, so the two
        // have identical layout and every bit pattern of one is valid for the
        // other. The crate provides no accessor for the value.
        other => MemoryKind::Unknown(unsafe { core::mem::transmute::<EntryType, u64>(other) }),
    }
}

/// The direct map's offset.
fn take_direct_map(request: &'static HhdmRequest) -> Result<DirectMap, HandoffError> {
    let response = request.get_response().ok_or(HandoffError::NoResponse)?;
    Ok(DirectMap::new(response.offset()))
}

fn take_framebuffer(request: &'static FramebufferRequest) -> Result<Framebuffer, FramebufferError> {
    let response = request.get_response().ok_or(FramebufferError::NoResponse)?;
    let fb = response
        .framebuffers()
        .next()
        .ok_or(FramebufferError::NoFramebuffer)?;

    if fb.memory_model() != MemoryModel::RGB {
        return Err(FramebufferError::UnsupportedMemoryModel);
    }
    if fb.bpp() != SUPPORTED_BPP {
        return Err(FramebufferError::UnsupportedBpp(fb.bpp()));
    }

    // `MmioRegion` requires 4-byte alignment for the 32-bit accesses this
    // framebuffer will see. Limine always satisfies both, but checking turns an
    // assumption inside an `unsafe` contract into a refusal we can report — the
    // alternative failure is every access being rejected and the screen simply
    // staying blank.
    if !fb.addr().addr().is_multiple_of(WORD) {
        return Err(FramebufferError::MisalignedAddress);
    }
    let pitch = fb.pitch() as usize;
    if !pitch.is_multiple_of(WORD) {
        return Err(FramebufferError::MisalignedPitch(pitch));
    }

    let info = FramebufferInfo {
        width: fb.width() as usize,
        height: fb.height() as usize,
        pitch,
        bytes_per_pixel: usize::from(fb.bpp()) / 8,
        format: PixelFormat::new(
            Channel::new(fb.red_mask_shift(), fb.red_mask_size()),
            Channel::new(fb.green_mask_shift(), fb.green_mask_size()),
            Channel::new(fb.blue_mask_shift(), fb.blue_mask_size()),
        ),
    };

    // SAFETY: the bootloader reports a live linear mapping of `pitch * height`
    // bytes that nothing else owns once it hands over control, and the checks
    // above confirmed the alignment `MmioRegion::new` requires.
    let region = unsafe { MmioRegion::new(fb.addr(), info.pitch * info.height) };

    Ok(Framebuffer::new(region, info))
}
