//! Typed x86 I/O port access.
//!
//! Ports are handed out by [`claim`] and [`claim_block`], which record every
//! claim in a registry so exclusivity is checked rather than merely promised.
//! Reads and writes are then ordinary safe calls, so a driver built on this
//! needs no `unsafe` of its own.
//!
//! A claim covers the ports an access *reaches*, not the one it names: an
//! `out dx, eax` to port `n` drives `n..n + 4`, so a `Port<u32>` reserves four.
//! The bookkeeping is `port_space::PortSpace`, where it is host-tested; this
//! module is the lock around it and the instructions underneath it.
//!
//! **A claim is permanent, and a caller that wants two must survive getting
//! one.** [`PortBlock`] and [`Port`] have no `Drop` and the registry never
//! releases — which is right for a driver that owns its registers for the life
//! of the kernel, and is a trap for a caller claiming a set. Take two blocks
//! and have the second refuse, and the first is claimed and stranded: nothing
//! can ever claim it again, so whatever needed both can never be built, on
//! this boot or any later attempt within it. `pic::silence` is the one caller
//! that does this today, and it names the consequence rather than guarding
//! against it, because on the one path it runs the ports are free (D-129).

use core::arch::asm;
use core::marker::PhantomData;

use port_space::PortSpace;

use crate::panic::PanicToken;
use crate::sync::SpinLock;

/// Which ports are spoken for. Claims are permanent; a driver owns its
/// registers for the life of the kernel.
static CLAIMED: SpinLock<PortSpace> = SpinLock::new(PortSpace::new());

/// Record `len` ports from `number` as claimed, all or nothing.
fn reserve(number: u16, len: usize) -> bool {
    CLAIMED.lock_leaf().reserve(number as usize, len)
}

/// Claim exclusive access to port `number` and the ports a `T`-wide access
/// reaches beyond it.
///
/// Returns `None` if any of them is already claimed, or if the access would
/// run past the end of I/O space.
pub fn claim<T: PortValue>(number: u16) -> Option<Port<T>> {
    reserve(number, size_of::<T>()).then(|| {
        // SAFETY: the registry has just recorded every port a `T`-wide access
        // at `number` drives, and no claim is ever released, so no other
        // `Port` or `PortBlock` can cover any of them.
        unsafe { Port::new(number) }
    })
}

/// Claim exclusive access to the `N` consecutive ports starting at `base`.
///
/// Returns `None` if any of them is already claimed, or if the block would run
/// past the end of I/O space.
pub fn claim_block<const N: usize>(base: u16) -> Option<PortBlock<N>> {
    reserve(base, N).then(|| {
        // SAFETY: the registry has just recorded every port in the block, and
        // no claim is ever released, so nothing else can cover them.
        unsafe { PortBlock::new(base) }
    })
}

/// Reclaim the registry's lock on the terminal panic path.
///
/// Called by [`panic::reclaim`](crate::panic::reclaim) rather than directly:
/// the panic path has to be able to claim a port to report through, and the
/// code it interrupted may have been part-way through a claim of its own.
pub(crate) fn force_unlock(panic: &PanicToken) {
    CLAIMED.force_unlock(panic);
}

mod sealed {
    #![allow(unreachable_pub)]

    pub trait Sealed {}
}

/// A width of value that can be transferred over an I/O port.
///
/// Sealed: the x86 port instructions exist only for 8, 16, and 32 bits, and
/// the only implementors are the primitives of exactly those widths. That is
/// what lets `size_of::<T>()` stand for the number of ports an access drives,
/// which is how [`claim`] knows how much to reserve.
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
    /// Deliberately not public: [`claim`] is the only door, so a handle cannot
    /// exist without the registry knowing about it.
    ///
    /// # Safety
    ///
    /// No other live `Port` or [`PortBlock`] may cover any of the
    /// `size_of::<T>()` ports starting at `number` — the span a `T`-wide
    /// access drives — and the caller must be entitled to drive whatever
    /// device answers there.
    const unsafe fn new(number: u16) -> Self {
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
/// Accesses are checked against `N` by the span they reach, not by the offset
/// they name, so a driver indexes its registers by name without being able to
/// walk off the end into another device — including with a `u16` or `u32`
/// access that starts inside the block and would finish outside it.
#[derive(Debug)]
pub struct PortBlock<const N: usize> {
    base: u16,
}

impl<const N: usize> PortBlock<N> {
    /// Claim the `N` ports starting at `base`.
    ///
    /// Deliberately not public: [`claim_block`] is the only door, so a handle
    /// cannot exist without the registry knowing about it.
    ///
    /// # Safety
    ///
    /// No other live [`Port`] or `PortBlock` may overlap `base..base + N`, and
    /// the caller must be entitled to drive the device that answers there.
    ///
    /// # Panics
    ///
    /// Panics if the block is empty or would run past port `0xFFFF`.
    const unsafe fn new(base: u16) -> Self {
        assert!(N > 0, "a port block covers at least one port");
        assert!(
            base as usize + N <= port_space::PORT_COUNT,
            "port block runs past the end of I/O space"
        );
        Self { base }
    }

    /// Read register `offset`.
    ///
    /// # Panics
    ///
    /// Panics unless the whole `T`-wide access lies within the block.
    pub fn read<T: PortValue>(&mut self, offset: usize) -> T {
        // SAFETY: `port_for` confirmed the whole access lies inside the block
        // this handle exclusively owns, and `&mut self` rules out a concurrent
        // access.
        unsafe { T::port_read(self.port_for::<T>(offset)) }
    }

    /// Write register `offset`.
    ///
    /// # Panics
    ///
    /// Panics unless the whole `T`-wide access lies within the block.
    pub fn write<T: PortValue>(&mut self, offset: usize, value: T) {
        // SAFETY: `port_for` confirmed the whole access lies inside the block
        // this handle exclusively owns, and `&mut self` rules out a concurrent
        // access.
        unsafe { T::port_write(self.port_for::<T>(offset), value) };
    }

    /// The port a `T`-wide access at `offset` starts on, having confirmed that
    /// every port it goes on to drive is inside the block.
    fn port_for<T: PortValue>(&self, offset: usize) -> u16 {
        let width = size_of::<T>();
        assert!(
            offset.checked_add(width).is_some_and(|end| end <= N),
            "a {width}-byte access at register {offset} leaves a {N}-port block"
        );
        // Cannot overflow: `new` bounded `base + N` to the end of I/O space and
        // the assertion above bounds `offset` below `N`.
        self.base + offset as u16
    }
}
