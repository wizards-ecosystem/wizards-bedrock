//! Typed x86 I/O port access.
//!
//! Ports are handed out by [`claim`] and [`claim_block`], which record every
//! claim in a registry so exclusivity is checked rather than merely promised.
//! Reads and writes are then ordinary safe calls, so a driver built on this
//! needs no `unsafe` of its own.

use core::arch::asm;
use core::marker::PhantomData;

use crate::sync::SpinLock;

/// Number of addressable I/O ports on x86.
const PORT_SPACE: usize = 0x1_0000;

/// One bit per port, set once that port has been claimed. Claims are permanent;
/// a driver owns its registers for the life of the kernel.
static CLAIMED: SpinLock<[u64; PORT_SPACE / 64]> = SpinLock::new([0; PORT_SPACE / 64]);

fn is_claimed(bitmap: &[u64], number: usize) -> bool {
    bitmap[number / 64] & (1 << (number % 64)) != 0
}

fn mark_claimed(bitmap: &mut [u64], number: usize) {
    bitmap[number / 64] |= 1 << (number % 64);
}

/// Claim exclusive access to port `number`.
///
/// Returns `None` if it is already claimed.
pub fn claim<T: PortValue>(number: u16) -> Option<Port<T>> {
    let mut claimed = CLAIMED.lock();
    if is_claimed(&claimed[..], number as usize) {
        return None;
    }
    mark_claimed(&mut claimed[..], number as usize);
    // SAFETY: the registry has just recorded this port, and no claim is ever
    // released, so no other `Port` or `PortBlock` can cover it.
    Some(unsafe { Port::new(number) })
}

/// Claim exclusive access to the `N` consecutive ports starting at `base`.
///
/// Returns `None` if any of them is already claimed, or if the block would run
/// past the end of I/O space.
pub fn claim_block<const N: usize>(base: u16) -> Option<PortBlock<N>> {
    let start = base as usize;
    let end = start.checked_add(N).filter(|&end| end <= PORT_SPACE)?;
    let mut claimed = CLAIMED.lock();
    if (start..end).any(|number| is_claimed(&claimed[..], number)) {
        return None;
    }
    for number in start..end {
        mark_claimed(&mut claimed[..], number);
    }
    // SAFETY: the registry has just recorded every port in the block, and no
    // claim is ever released, so nothing else can cover them.
    Some(unsafe { PortBlock::new(base) })
}

mod sealed {
    #![allow(unreachable_pub)]

    pub trait Sealed {}
}

/// A width of value that can be transferred over an I/O port.
///
/// Sealed: the x86 port instructions exist only for 8, 16, and 32 bits.
pub trait PortValue: sealed::Sealed + Copy {
    /// Read one value from `port`.
    ///
    /// # Safety
    ///
    /// The caller must own `port`. Reads from device ports are not necessarily
    /// pure — many advance a FIFO or clear a status latch as a side effect.
    unsafe fn port_read(port: u16) -> Self;

    /// Write one value to `port`.
    ///
    /// # Safety
    ///
    /// The caller must own `port`. A port write drives hardware directly and
    /// can have arbitrary effects on the machine.
    unsafe fn port_write(port: u16, value: Self);
}

impl sealed::Sealed for u8 {}

impl PortValue for u8 {
    unsafe fn port_read(port: u16) -> Self {
        let value: u8;
        // SAFETY: `in al, dx` reads one byte from the port in `dx`; the caller
        // guarantees the port is theirs and the read's side effects are wanted.
        unsafe {
            asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags));
        }
        value
    }

    unsafe fn port_write(port: u16, value: Self) {
        // SAFETY: `out dx, al` writes one byte to the port in `dx`; the caller
        // guarantees the port is theirs to drive.
        unsafe {
            asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
        }
    }
}

impl sealed::Sealed for u16 {}

impl PortValue for u16 {
    unsafe fn port_read(port: u16) -> Self {
        let value: u16;
        // SAFETY: `in ax, dx` reads one word from the port in `dx`; the caller
        // guarantees the port is theirs and the read's side effects are wanted.
        unsafe {
            asm!("in ax, dx", in("dx") port, out("ax") value, options(nomem, nostack, preserves_flags));
        }
        value
    }

    unsafe fn port_write(port: u16, value: Self) {
        // SAFETY: `out dx, ax` writes one word to the port in `dx`; the caller
        // guarantees the port is theirs to drive.
        unsafe {
            asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
        }
    }
}

impl sealed::Sealed for u32 {}

impl PortValue for u32 {
    unsafe fn port_read(port: u16) -> Self {
        let value: u32;
        // SAFETY: `in eax, dx` reads one dword from the port in `dx`; the caller
        // guarantees the port is theirs and the read's side effects are wanted.
        unsafe {
            asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags));
        }
        value
    }

    unsafe fn port_write(port: u16, value: Self) {
        // SAFETY: `out dx, eax` writes one dword to the port in `dx`; the caller
        // guarantees the port is theirs to drive.
        unsafe {
            asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
        }
    }
}

/// Exclusive access to a single I/O port.
#[derive(Debug)]
pub struct Port<T: PortValue> {
    number: u16,
    value: PhantomData<T>,
}

impl<T: PortValue> Port<T> {
    /// Claim `number`.
    ///
    /// # Safety
    ///
    /// No other live `Port` or [`PortBlock`] may cover `number`, and the caller
    /// must be entitled to drive whatever device answers there.
    pub const unsafe fn new(number: u16) -> Self {
        Self {
            number,
            value: PhantomData,
        }
    }

    /// The port number this handle covers.
    pub const fn number(&self) -> u16 {
        self.number
    }

    /// Read one value from the port.
    pub fn read(&mut self) -> T {
        // SAFETY: constructing this `Port` asserted exclusive entitlement to
        // `self.number`, and `&mut self` rules out a concurrent access.
        unsafe { T::port_read(self.number) }
    }

    /// Write one value to the port.
    pub fn write(&mut self, value: T) {
        // SAFETY: constructing this `Port` asserted exclusive entitlement to
        // `self.number`, and `&mut self` rules out a concurrent access.
        unsafe { T::port_write(self.number, value) };
    }
}

/// Exclusive access to `N` consecutive I/O ports — a device's register file.
///
/// Register offsets are checked against `N`, so a driver indexes its registers
/// by name without being able to walk off the end into another device.
#[derive(Debug)]
pub struct PortBlock<const N: usize> {
    base: u16,
}

impl<const N: usize> PortBlock<N> {
    /// Claim the `N` ports starting at `base`.
    ///
    /// # Safety
    ///
    /// No other live [`Port`] or `PortBlock` may overlap `base..base + N`, and
    /// the caller must be entitled to drive the device that answers there.
    ///
    /// # Panics
    ///
    /// Panics if the block would run past port `0xFFFF`.
    pub const unsafe fn new(base: u16) -> Self {
        assert!(
            base as usize + N <= 0x1_0000,
            "port block runs past the end of I/O space"
        );
        Self { base }
    }

    /// Read register `offset`.
    ///
    /// # Panics
    ///
    /// Panics if `offset` is not within the block.
    pub fn read<T: PortValue>(&mut self, offset: usize) -> T {
        // SAFETY: the offset is inside the block this handle exclusively owns,
        // and `&mut self` rules out a concurrent access.
        unsafe { T::port_read(self.port_for(offset)) }
    }

    /// Write register `offset`.
    ///
    /// # Panics
    ///
    /// Panics if `offset` is not within the block.
    pub fn write<T: PortValue>(&mut self, offset: usize, value: T) {
        // SAFETY: the offset is inside the block this handle exclusively owns,
        // and `&mut self` rules out a concurrent access.
        unsafe { T::port_write(self.port_for(offset), value) };
    }

    fn port_for(&self, offset: usize) -> u16 {
        assert!(offset < N, "register {offset} is outside a {N}-port block");
        self.base + offset as u16
    }
}
