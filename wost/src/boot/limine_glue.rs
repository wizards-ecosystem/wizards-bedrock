//! The only file in the kernel that names the bootloader protocol.

use limine::BaseRevision;
use limine::framebuffer::MemoryModel;
use limine::memory_map::EntryType;
use limine::request::{
    ExecutableAddressRequest, FramebufferRequest, HhdmRequest, MemoryMapRequest, ModuleRequest,
    RsdpRequest,
};

use pixel::{Channel, FramebufferLayout, PixelFormat};

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

/// Assemble [`BootInfo`] from the bootloader's answers and hand over control.
///
/// Called only by [`entry!`](crate::entry).
pub fn enter(requests: Requests, kernel_main: fn(BootInfo) -> !) -> ! {
    let Requests {
        base_revision,
        framebuffer,
        memory_map,
        hhdm,
        image,
        rsdp,
        modules,
    } = requests;
    // A second caller would mint a second framebuffer over live device memory.
    static ENTERED: crate::sync::Once = crate::sync::Once::new();
    assert!(ENTERED.claim(), "the kernel has already been entered");

    kernel_main(BootInfo {
        base_revision_supported: base_revision.is_supported(),
        framebuffer: take_framebuffer(framebuffer),
        memory_map: take_memory_map(memory_map),
        direct_map: take_direct_map(hhdm),
        image: take_image(image),
        rsdp: take_rsdp(rsdp),
        module: take_module(modules),
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

/// The first module the bootloader loaded, as bytes.
///
/// One, not all: the initrd is a single program (D-100), and an API that
/// returned a list would be shaped for a filesystem this kernel does not have.
/// The slice borrows memory the bootloader owns, which is why it is `'static`
/// and why nothing may write it — it lives in a reclaimable region that stays
/// mapped for the life of the boot.
fn take_module(request: &'static ModuleRequest) -> Result<&'static [u8], HandoffError> {
    let response = request.get_response().ok_or(HandoffError::NoResponse)?;
    let module = response.modules().first().ok_or(HandoffError::Empty)?;
    let address = module.addr();
    let size = module.size();
    if address.is_null() || size == 0 {
        return Err(HandoffError::Empty);
    }
    // `from_raw_parts` states two bounds the bootloader's numbers do not
    // promise: the length must fit an `isize`, and the range must not wrap.
    // Neither is reachable from a real Limine, and neither was checked, which
    // is the difference the audit was about (D-105).
    let len = usize::try_from(size)
        .ok()
        .filter(|len| *len <= isize::MAX as usize)
        .ok_or(HandoffError::Empty)?;
    if (address as usize).checked_add(len).is_none() {
        return Err(HandoffError::Empty);
    }
    // SAFETY: the bootloader reports this range as a module it loaded and
    // keeps mapped, and the protocol makes it ours to read for the life of the
    // boot. Nothing else holds a reference to it, and it is never written.
    Ok(unsafe { core::slice::from_raw_parts(address.cast_const(), len) })
}

/// The ACPI root pointer's address.
fn take_rsdp(request: &'static RsdpRequest) -> Result<u64, HandoffError> {
    let response = request.get_response().ok_or(HandoffError::NoResponse)?;
    match response.address() as u64 {
        0 => Err(HandoffError::Empty),
        address => Ok(address),
    }
}

/// Every request the entry glue places, gathered so the handoff is one
/// argument rather than a list that grows with each protocol feature.
///
/// The fields are the bootloader's types, which is why this lives here: it is
/// the only file that names them (`005`, and swapping bootloaders touches one
/// file).
///
/// No `Debug`, unusually for this codebase (`005`, C-DEBUG): the bootloader's
/// request types do not implement it, and a hand-written impl here would print
/// the addresses of static requests, which is not information anyone wants and
/// is exactly what a KASLR ledger would rather not see in a log.
#[expect(missing_debug_implementations)]
#[derive(Clone, Copy)]
pub struct Requests {
    /// The protocol revision the kernel asked for.
    pub base_revision: &'static BaseRevision,
    /// The framebuffer, if the firmware gave one.
    pub framebuffer: &'static FramebufferRequest,
    /// The memory map.
    pub memory_map: &'static MemoryMapRequest,
    /// The direct map's offset.
    pub hhdm: &'static HhdmRequest,
    /// Where the kernel image was placed.
    pub image: &'static ExecutableAddressRequest,
    /// The ACPI root pointer.
    pub rsdp: &'static RsdpRequest,
    /// The modules, of which this kernel reads the first (D-100).
    pub modules: &'static ModuleRequest,
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

    let pitch = fb.pitch() as usize;
    let width = fb.width() as usize;
    let height = fb.height() as usize;
    let bytes_per_pixel = usize::from(fb.bpp()) / 8;
    let layout = FramebufferLayout::new(fb.addr().addr(), width, height, pitch, bytes_per_pixel)
        .map_err(FramebufferError::InvalidGeometry)?;
    let format = PixelFormat::new(
        Channel::new(fb.red_mask_shift(), fb.red_mask_size()),
        Channel::new(fb.green_mask_shift(), fb.green_mask_size()),
        Channel::new(fb.blue_mask_shift(), fb.blue_mask_size()),
    )
    .map_err(FramebufferError::InvalidPixelFormat)?;
    let info = FramebufferInfo {
        width,
        height,
        pitch,
        bytes_per_pixel,
        format,
    };

    // SAFETY: the bootloader reports a live linear mapping that nothing else
    // owns after handoff, and `layout` checked its span and alignment.
    let region = unsafe { MmioRegion::new(fb.addr(), layout.span()) };

    Ok(Framebuffer::new(region, info, layout))
}
