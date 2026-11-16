use core::arch::asm;
use core::marker::PhantomData;

use port_space::PortSpace;

use crate::panic::PanicToken;
use crate::sync::SpinLock;

// Fixed claims are permanent, so a register set is claimed in one transaction.
static CLAIMED: SpinLock<PortSpace> = SpinLock::new(PortSpace::new());

fn reserve(number: u16, len: usize) -> bool {
    CLAIMED.lock_leaf().reserve(number as usize, len)
}

// A claim covers every port an access reaches: `out dx, eax` to port `n` drives `n..n + 4`.
pub fn claim<T: PortValue>(number: u16) -> Option<Port<T>> {
    reserve(number, size_of::<T>()).then(|| {
        // SAFETY: the registry has just recorded every port a `T`-wide access at `number` drives,
        // and no claim is ever released, so no other `Port` or `PortBlock` covers one.
        unsafe { Port::new(number) }
    })
}

pub(crate) fn claim_pair<T: PortValue, U: PortValue>(
    first: u16,
    second: u16,
) -> Option<(Port<T>, Port<U>)> {
    let spans = [
        (first as usize, size_of::<T>()),
        (second as usize, size_of::<U>()),
    ];
    if !CLAIMED.lock_leaf().reserve_all(&spans) {
        return None;
    }
    // SAFETY: the registry has just recorded every port both accesses drive, all or none, and no
    // claim is ever released, so no other `Port` or `PortBlock` covers any of them.
    let first = unsafe { Port::new(first) };
    // SAFETY: as above; the same reservation covered this port, and `reserve_all` refused the
    // pair had the two overlapped.
    let second = unsafe { Port::new(second) };
    Some((first, second))
}

pub(crate) fn claim_block<const N: usize>(base: u16) -> Option<PortBlock<N>> {
    reserve(base, N).then(|| {
        // SAFETY: the registry has just recorded every port in the block, and no claim is ever
        // released, so no other `Port` or `PortBlock` covers them.
        unsafe { PortBlock::new(base) }
    })
}

pub(crate) fn claim_block_pair<const N: usize, const M: usize>(
    first: u16,
    second: u16,
) -> Option<(PortBlock<N>, PortBlock<M>)> {
    let spans = [(first as usize, N), (second as usize, M)];
    if !CLAIMED.lock_leaf().reserve_all(&spans) {
        return None;
    }
    // SAFETY: the registry has just recorded every port in both blocks, all or none, and no
    // claim is ever released, so no other `Port` or `PortBlock` covers any of them.
    let first = unsafe { PortBlock::new(first) };
    // SAFETY: as above; `reserve_all` refused the pair had the two overlapped.
    let second = unsafe { PortBlock::new(second) };
    Some((first, second))
}

#[cfg(feature = "selftest")]
pub fn debug_exit() -> Option<Port<u32>> {
    claim(0xF4)
}

pub(crate) fn isa_dma_controllers() -> Option<(PortBlock<16>, PortBlock<32>)> {
    claim_block_pair(0x00, 0xC0)
}

pub fn i8042() -> Option<(Port<u8>, Port<u8>)> {
    claim_pair(0x60, 0x64)
}

pub fn pit() -> Option<PortBlock<4>> {
    claim_block(0x40)
}

pub fn com1() -> Option<PortBlock<8>> {
    claim_block(0x3F8)
}

pub(crate) fn pci_configuration() -> Option<(Port<u32>, Port<u32>)> {
    claim_pair(0x0CF8, 0x0CFC)
}

pub(crate) fn force_unlock(panic: &PanicToken) {
    CLAIMED.force_unlock(panic);
}

mod sealed {
    pub trait Sealed {}
}

// Sealed to the port instruction widths, so `size_of::<T>()` is the ports an access drives.
pub trait PortValue: sealed::Sealed + Copy {
    /// # Safety
    ///
    /// The caller must own `port`, and want the read's side effects: many advance a FIFO or clear
    /// a status latch.
    unsafe fn port_read(port: u16) -> Self;

    /// # Safety
    ///
    /// The caller must own `port`; a port write can have arbitrary effects on the machine.
    unsafe fn port_write(port: u16, value: Self);
}

impl sealed::Sealed for u8 {}

impl PortValue for u8 {
    unsafe fn port_read(port: u16) -> Self {
        let value: Self;
        // SAFETY: `in al, dx` reads one byte from the port in `dx`; the caller guarantees the port
        // is theirs and the read's side effects are wanted.
        unsafe {
            asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags));
        }
        value
    }

    unsafe fn port_write(port: u16, value: Self) {
        // SAFETY: `out dx, al` writes one byte to the port in `dx`; the caller guarantees the port
        // is theirs to drive.
        unsafe {
            asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
        }
    }
}

impl sealed::Sealed for u16 {}

impl PortValue for u16 {
    unsafe fn port_read(port: u16) -> Self {
        let value: Self;
        // SAFETY: `in ax, dx` reads one word from the port in `dx`; the caller guarantees the port
        // is theirs and the read's side effects are wanted.
        unsafe {
            asm!("in ax, dx", in("dx") port, out("ax") value, options(nomem, nostack, preserves_flags));
        }
        value
    }

    unsafe fn port_write(port: u16, value: Self) {
        // SAFETY: `out dx, ax` writes one word to the port in `dx`; the caller guarantees the port
        // is theirs to drive.
        unsafe {
            asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
        }
    }
}

impl sealed::Sealed for u32 {}

impl PortValue for u32 {
    unsafe fn port_read(port: u16) -> Self {
        let value: Self;
        // SAFETY: `in eax, dx` reads one dword from the port in `dx`; the caller guarantees the
        // port is theirs and the read's side effects are wanted.
        unsafe {
            asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags));
        }
        value
    }

    unsafe fn port_write(port: u16, value: Self) {
        // SAFETY: `out dx, eax` writes one dword to the port in `dx`; the caller guarantees the
        // port is theirs to drive.
        unsafe {
            asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
        }
    }
}

#[derive(Debug)]
pub struct Port<T: PortValue> {
    number: u16,
    value: PhantomData<T>,
}

impl<T: PortValue> Port<T> {
    /// # Safety
    ///
    /// No other live `Port` or [`PortBlock`] may cover any of the `size_of::<T>()` ports a `T`-wide
    /// access at `number` drives, and the caller must be entitled to drive the device there.
    const unsafe fn new(number: u16) -> Self {
        Self {
            number,
            value: PhantomData,
        }
    }

    pub fn read(&mut self) -> T {
        // SAFETY: constructing this `Port` asserted exclusive entitlement to `self.number`, and
        // `&mut self` rules out a concurrent access.
        unsafe { <T as PortValue>::port_read(self.number) }
    }

    pub fn write(&mut self, value: T) {
        // SAFETY: constructing this `Port` asserted exclusive entitlement to `self.number`, and
        // `&mut self` rules out a concurrent access.
        unsafe { <T as PortValue>::port_write(self.number, value) };
    }
}

#[derive(Debug)]
pub struct PortBlock<const N: usize> {
    base: u16,
}

impl<const N: usize> PortBlock<N> {
    /// # Safety
    ///
    /// No other live [`Port`] or `PortBlock` may overlap `base..base + N`, and the caller must be
    /// entitled to drive the device that answers there.
    const unsafe fn new(base: u16) -> Self {
        assert!(N > 0, "a port block covers at least one port");
        assert!(
            base as usize + N <= port_space::PORT_COUNT,
            "port block runs past the end of I/O space"
        );
        Self { base }
    }

    pub fn read<T: PortValue>(&mut self, offset: usize) -> T {
        // SAFETY: `port_for` confirmed the whole access lies inside the block this handle
        // exclusively owns, and `&mut self` rules out a concurrent access.
        unsafe { <T as PortValue>::port_read(self.port_for::<T>(offset)) }
    }

    pub fn write<T: PortValue>(&mut self, offset: usize, value: T) {
        // SAFETY: `port_for` confirmed the whole access lies inside the block this handle
        // exclusively owns, and `&mut self` rules out a concurrent access.
        unsafe { <T as PortValue>::port_write(self.port_for::<T>(offset), value) };
    }

    fn port_for<T: PortValue>(&self, offset: usize) -> u16 {
        let width = size_of::<T>();
        assert!(
            offset.checked_add(width).is_some_and(|end| end <= N),
            "a {width}-byte access at register {offset} leaves a {N}-port block"
        );
        // `new` bounded `base + N` and the assertion keeps `offset + width <= N`: no overflow.
        self.base + offset as u16
    }
}
