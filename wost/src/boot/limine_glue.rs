//! The only file in the kernel that names the bootloader protocol.

use limine::BaseRevision;
use limine::framebuffer::MemoryModel;
use limine::request::FramebufferRequest;

use pixel::{Channel, PixelFormat};

use crate::boot::{BootInfo, Framebuffer, FramebufferError, FramebufferInfo};
use crate::mmio::MmioRegion;

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
    kernel_main: fn(BootInfo) -> !,
) -> ! {
    kernel_main(BootInfo {
        base_revision_supported: base_revision.is_supported(),
        framebuffer: take_framebuffer(framebuffer),
    })
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

    Ok(Framebuffer { region, info })
}
