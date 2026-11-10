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

/// Boot protocol revision 4, which admits device holes and maps ACPI tables.
///
/// A function because a `const` of `BaseRevision` would hold `UnsafeCell`s.
pub const fn base_revision() -> BaseRevision {
    BaseRevision::with_revision(4)
}

const SUPPORTED_BPP: u16 = 32;

/// Assemble [`BootInfo`] from the bootloader's answers and hand over control.
///
/// Called only by [`entry!`](crate::entry).
pub fn enter(requests: Requests, kernel_main: fn(BootInfo) -> !) -> ! {
    let Requests {
        base_revision,
        framebuffer,
        memory_map,
        direct_map,
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
        direct_map: take_direct_map(direct_map),
        image: take_image(image),
        rsdp: take_rsdp(rsdp),
        module: take_module(modules),
    })
}

fn take_image(request: &'static ExecutableAddressRequest) -> Result<ImageAddress, HandoffError> {
    let response = request.get_response().ok_or(HandoffError::NoResponse)?;
    Ok(ImageAddress::new(
        response.physical_base(),
        response.virtual_base(),
    ))
}

/// The first module, the one-program initrd, in a reclaimable region kept mapped; never write it.
fn take_module(request: &'static ModuleRequest) -> Result<&'static [u8], HandoffError> {
    let response = request.get_response().ok_or(HandoffError::NoResponse)?;
    let module = response.modules().first().ok_or(HandoffError::Empty)?;
    let address = module.addr();
    let size = module.size();
    if address.is_null() || size == 0 {
        return Err(HandoffError::Empty);
    }
    // `from_raw_parts` also needs an `isize` length and a non-wrapping range the handoff lacks.
    let len = usize::try_from(size)
        .ok()
        .filter(|len| *len <= isize::MAX as usize)
        .ok_or(HandoffError::Empty)?;
    if (address as usize).checked_add(len).is_none() {
        return Err(HandoffError::Empty);
    }
    // SAFETY: the bootloader keeps this loaded module mapped and ours to read for the whole boot;
    // nothing else holds a reference to it, and it is never written.
    Ok(unsafe { core::slice::from_raw_parts(address.cast_const(), len) })
}

fn take_rsdp(request: &'static RsdpRequest) -> Result<u64, HandoffError> {
    let response = request.get_response().ok_or(HandoffError::NoResponse)?;
    match response.address() as u64 {
        0 => Err(HandoffError::Empty),
        address => Ok(address),
    }
}

/// Every request the entry glue places, gathered so the handoff is one argument.
#[expect(
    missing_debug_implementations,
    reason = "the bootloader's request types have no Debug, and a hand-written one would only print static addresses, which stay out of logs"
)]
#[derive(Clone, Copy)]
pub struct Requests {
    pub base_revision: &'static BaseRevision,
    pub framebuffer: &'static FramebufferRequest,
    pub memory_map: &'static MemoryMapRequest,
    pub direct_map: &'static HhdmRequest,
    pub image: &'static ExecutableAddressRequest,
    pub rsdp: &'static RsdpRequest,
    pub modules: &'static ModuleRequest,
}

fn take_memory_map(request: &'static MemoryMapRequest) -> Result<MemoryMap, HandoffError> {
    let response = request.get_response().ok_or(HandoffError::NoResponse)?;
    let entries = response.entries();
    MemoryMap::try_from_regions(entries.iter().map(|entry| MemoryRegion {
        base: entry.base,
        len: entry.length,
        kind: kind_of(entry.entry_type),
    }))
    .map_err(HandoffError::MemoryMap)
}

/// Translate one entry type by the crate's own constants, so a moved value fails to compile.
///
/// An unknown type keeps its raw value for the log and never folds into `Reserved`.
/// Base revision 4 adds `RESERVED_MAPPED`, which the pinned 0.5.0 crate has no constant for.
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
        // SAFETY: `EntryType` is `#[repr(transparent)]` over `u64`: identical layout, every bit
        // pattern valid for both. The crate provides no accessor for the value.
        other => MemoryKind::Unknown(unsafe { core::mem::transmute::<EntryType, u64>(other) }),
    }
}

fn take_direct_map(request: &'static HhdmRequest) -> Result<DirectMap, HandoffError> {
    let response = request.get_response().ok_or(HandoffError::NoResponse)?;
    Ok(DirectMap::new(response.offset()))
}

fn take_framebuffer(request: &'static FramebufferRequest) -> Result<Framebuffer, FramebufferError> {
    let response = request.get_response().ok_or(FramebufferError::NoResponse)?;
    let fb = core::iter::Iterator::next(&mut response.framebuffers())
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

    // SAFETY: nothing owns the live mapping after handoff, and `layout` checked span and alignment;
    // paging activation verifies it before the switch; later roots share the frozen higher half.
    let region = unsafe { MmioRegion::new(fb.addr(), layout.span()) };

    Ok(Framebuffer::new(region, info, layout))
}
