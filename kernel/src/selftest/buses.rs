use core::sync::atomic::{AtomicUsize, Ordering};

use super::threads::context;
use crate::pci;

pub(super) fn the_bus_walk_descends_through_a_bridge() -> bool {
    let mut on_bus_zero = 0;
    let mut beyond = 0;
    let Some(survey) = pci::enumerate(|function| {
        if function.at.bus() == 0 {
            on_bus_zero += 1;
        } else {
            beyond += 1;
        }
    }) else {
        return false;
    };

    // More than one bus visited, and a bridge to explain why.
    survey.buses >= 2
        && survey.bridges >= 1
        && survey.highest_bus >= 1
        // And the counts agree with what the callback saw, so the survey is
        // reporting the walk rather than a number of its own.
        && survey.functions == on_bus_zero + beyond
        && on_bus_zero >= 6
}

static DEVICE_CLAIM_CHECK: AtomicUsize = AtomicUsize::new(0);
static DEVICE_PORT_CHECK: AtomicUsize = AtomicUsize::new(0);

/// Run the two checks that require a free PCI function before VT-d requester
/// takeover owns the complete unit. Each path returns every temporary owner;
/// only the immutable verdict crosses into the later report.
pub(crate) fn prepare_pci_claim_checks() {
    if DEVICE_CLAIM_CHECK.load(Ordering::Acquire) != 0 {
        return;
    }
    let claim = check_device_is_claimed_once_and_a_refusal_strands_nothing();
    let ports = check_claimed_devices_io_window_is_not_claimable_bare();
    DEVICE_CLAIM_CHECK.store(usize::from(claim) + 1, Ordering::Release);
    DEVICE_PORT_CHECK.store(usize::from(ports) + 1, Ordering::Release);
}

pub(super) fn a_device_is_claimed_once_and_a_refusal_strands_nothing() -> bool {
    DEVICE_CLAIM_CHECK.load(Ordering::Acquire) == 2
}

fn check_device_is_claimed_once_and_a_refusal_strands_nothing() -> bool {
    // The bridge behind bus zero: it has no windows to disturb, and it is
    // there on both boot targets.
    let mut bridge = None;
    if pci::enumerate(|function| {
        if bridge.is_none() && function.class.is_bridge() && function.header.is_bridge() {
            bridge = Some(function.at);
        }
    })
    .is_none()
    {
        return false;
    }
    let Some(at) = bridge else {
        return false;
    };

    let before = pci::claims();
    let Some(alias) = at.at(pci_config::header::CLASS) else {
        return false;
    };
    pci::refuse_clean_preparation_for_test();
    let clean_refusal = matches!(pci::lease(alias), Err(pci::ClaimError::WindowTaken))
        && !pci::is_claimed(at)
        && pci::claims() == before;
    let Ok(held) = pci::lease(alias) else {
        return false;
    };
    let recorded = pci::is_claimed(at) && pci::claims() == before + 1;

    // A different register offset is still the same function and the same
    // claim, so a second driver cannot enter through an alias.
    let refused = matches!(
        pci::lease(at.at(pci_config::header::BAR0).expect("a BAR offset")),
        Err(pci::ClaimError::Taken)
    );

    // And the holder still holds it: a refusal that released what it found
    // would be indistinguishable from here without asking.
    let intact = pci::is_claimed(at)
        && pci::claims() == before + 1
        && held.at() == at.function_base()
        && held.identity().0 != 0xFFFF;

    // Bridge headers have two BARs; probing a third would overwrite bus numbers
    // before restoring them (D-168).
    let two_registers = held.window(2).is_none()
        && held.window(3).is_none()
        && held.window(4).is_none()
        && held.window(5).is_none();

    let released = held.release(&mut context()).into_retained().is_none()
        && !pci::is_claimed(at)
        && pci::claims() == before;

    clean_refusal && recorded && refused && intact && two_registers && released
}

pub(super) fn a_claimed_devices_io_window_is_not_claimable_bare() -> bool {
    DEVICE_PORT_CHECK.load(Ordering::Acquire) == 2
}

fn check_claimed_devices_io_window_is_not_claimable_bare() -> bool {
    let claims_before = pci::claims();
    let mut candidate = None;
    if pci::enumerate(|function| {
        if candidate.is_none() && !function.header.is_bridge() && !pci::is_claimed(function.at) {
            candidate = Some(*function);
        }
    })
    .is_none()
    {
        return false;
    }

    // Walk the candidates until one with an I/O window is claimed; none
    // having one is a failure here, not a pass.
    let mut context = context();
    let mut found = None;
    let mut seen = 0;
    let mut retirement_clean = true;
    let surveyed = pci::enumerate(|function| {
        if found.is_some() || function.header.is_bridge() || pci::is_claimed(function.at) {
            return;
        }
        seen += 1;
        let Ok(device) = pci::lease(function.at) else {
            return;
        };
        let mut io = None;
        for index in 0..pci::BARS {
            if let Some(window) = device.window(index)
                && let pci_config::Bar::Io { port } = window.bar
                && window.size > 0
            {
                io = Some((port, window.size));
                break;
            }
        }
        if let Some((port, size)) = io {
            found = Some((device, port, size));
        } else {
            retirement_clean &= device.release(&mut context).into_retained().is_none();
        }
    })
    .is_some();

    let Some((device, port, size)) = found else {
        return false;
    };

    // Every port in the window is the device's. The first and the last, so a
    // claim that recorded only the base would be caught.
    let last = port + (size as u16 - 1);
    let base_refused = wost::arch::current::port::claim::<u8>(port).is_none();
    let last_refused = wost::arch::current::port::claim::<u8>(last).is_none();

    // Reacquiring the same function exercises both registries, then returns
    // it so the later unit-wide requester takeover can own the exact function.
    let at = device.at();
    wost::arch::current::port::refuse_span_release_for_test();
    let (reason, device) = match device.release(&mut context).into_retained() {
        None => return false,
        Some(retained) => retained,
    };
    retirement_clean &= reason == pci::ReleaseError::Ports
        && pci::is_claimed(at)
        && pci::claims() == claims_before + 1
        && wost::arch::current::port::claim::<u8>(port).is_none();
    retirement_clean &= device.release(&mut context).into_retained().is_none();
    let reclaimed = match pci::lease(at) {
        Ok(reclaimed) => {
            let same_window = (0..pci::BARS).any(|index| {
                reclaimed.window(index).is_some_and(|window| {
                    window.bar == pci_config::Bar::Io { port } && window.size == size
                })
            });
            same_window && reclaimed.release(&mut context).into_retained().is_none()
        }
        Err(_) => false,
    };

    // An outside claim proves refusal is limited to the device's window.
    let outside = port.checked_add(size as u16).is_some_and(|outside| {
        wost::arch::current::port::claim_spans::<1>(&[(outside, 1)])
            .is_some_and(|span| wost::arch::current::port::release_spans(span).is_ok())
    });

    surveyed && seen > 0 && base_refused && last_refused && retirement_clean && reclaimed && outside
}
