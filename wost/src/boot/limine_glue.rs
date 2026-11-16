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
    ImageAddress, MemoryKind, MemoryMap, MemoryRegion, record_framebuffer,
};
use crate::mmio::MmioRegion;

// A function because a `const` of `BaseRevision` would hold `UnsafeCell`s.
pub const fn base_revision() -> BaseRevision {
    BaseRevision::with_revision(4)
}

const SUPPORTED_BPP: u16 = 32;

pub fn enter(requests: Requests, kernel_main: fn(BootInfo) -> !) -> ! {
    // A second caller would mint a second framebuffer over live device memory.
    static ENTERED: crate::sync::Once = crate::sync::Once::new();
    assert!(ENTERED.claim(), "the kernel has already been entered");
    let Requests {
        base_revision,
        framebuffer,
        memory_map,
        direct_map,
        image,
        rsdp,
        modules,
    } = requests;

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
    // SAFETY: the module lies in `KernelAndModules` memory, which is never allocatable and which the
    // bootloader's direct map and ours (verified at activation) both cover; nothing writes it.
    Ok(unsafe { core::slice::from_raw_parts(address.cast_const(), len) })
}

fn take_rsdp(request: &'static RsdpRequest) -> Result<u64, HandoffError> {
    let response = request.get_response().ok_or(HandoffError::NoResponse)?;
    match response.address() as u64 {
        0 => Err(HandoffError::Empty),
        address => Ok(address),
    }
}

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

// Revision 4's `RESERVED_MAPPED` has no constant in the pinned crate; unknown types keep theirs.
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
        // SAFETY: `transmute` refuses to compile unless the sizes match, and every bit pattern of
        // that size is a valid `u64`; the crate offers no accessor for the raw value.
        other => MemoryKind::Other(unsafe { core::mem::transmute::<EntryType, u64>(other) }),
    }
}

fn take_direct_map(request: &'static HhdmRequest) -> Result<DirectMap, HandoffError> {
    let response = request.get_response().ok_or(HandoffError::NoResponse)?;
    DirectMap::from_handoff(response.offset())
}

fn take_framebuffer(request: &'static FramebufferRequest) -> Result<Framebuffer, FramebufferError> {
    let response = request.get_response().ok_or(FramebufferError::NoResponse)?;
    let framebuffer = response
        .framebuffers()
        .next()
        .ok_or(FramebufferError::NoFramebuffer)?;

    if framebuffer.memory_model() != MemoryModel::RGB {
        return Err(FramebufferError::UnsupportedMemoryModel);
    }
    if framebuffer.bpp() != SUPPORTED_BPP {
        return Err(FramebufferError::UnsupportedBpp(framebuffer.bpp()));
    }

    let pitch = framebuffer.pitch() as usize;
    let width = framebuffer.width() as usize;
    let height = framebuffer.height() as usize;
    let bytes_per_pixel = usize::from(framebuffer.bpp()) / 8;
    let layout = FramebufferLayout::new(
        framebuffer.addr().addr(),
        width,
        height,
        pitch,
        bytes_per_pixel,
    )
    .map_err(FramebufferError::InvalidGeometry)?;
    let format = PixelFormat::new(
        Channel::new(framebuffer.red_mask_shift(), framebuffer.red_mask_size()),
        Channel::new(
            framebuffer.green_mask_shift(),
            framebuffer.green_mask_size(),
        ),
        Channel::new(framebuffer.blue_mask_shift(), framebuffer.blue_mask_size()),
    )
    .map_err(FramebufferError::InvalidPixelFormat)?;
    let info = FramebufferInfo {
        width,
        height,
        pitch,
        bytes_per_pixel,
        format,
    };

    record_framebuffer(framebuffer.addr().addr() as u64, layout.span() as u64);
    // SAFETY: the live mapping has no other owner after handoff; `layout` checked span and align;
    // activation verifies this recorded extent first; user roots share the frozen higher half.
    let region = unsafe { MmioRegion::new(framebuffer.addr(), layout.span()) };

    Ok(Framebuffer::new(region, info, layout))
}
