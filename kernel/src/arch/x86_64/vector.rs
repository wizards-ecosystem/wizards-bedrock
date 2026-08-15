//! Which interrupt vector carries what.
//!
//! Vectors 0–31 belong to the architecture and are named in
//! `x86_tables::exception`. The kernel owns the rest, and this module is the
//! whole of that decision — one place to look for a collision, and one place
//! that grows when a device wants a line.
//!
//! The two the local APIC needs for itself are the highest two, so that device
//! vectors can be handed out upward from 32 without ever meeting them.

use wost::arch::current::apic::Vectors;

/// Delivered when the local APIC has committed to delivering something and then
/// found it had nothing to say.
///
/// The low four bits are all set because the earliest APICs hardwire them, so
/// any other value would silently become this one anyway.
pub(crate) const SPURIOUS: u8 = 0xFF;

/// Delivered when the local APIC records an error against itself.
pub(crate) const ERROR: u8 = 0xFE;

/// The kernel sends this to itself.
///
/// It exists to make the interrupt path testable end to end — controller
/// configured, gate reached, frame saved, interrupt dismissed — which is
/// otherwise unobservable until there is a device generating interrupts. It is
/// also the mechanism a second CPU will be asked to do work through, so it is
/// not going away when the timer arrives.
pub(crate) const PING: u8 = 0x40;

/// What the local APIC is told to use for itself.
pub(crate) const LOCAL_APIC: Vectors = Vectors {
    spurious: SPURIOUS,
    error: ERROR,
};
