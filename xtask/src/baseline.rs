//! Repeating the fixed-QEMU foundation baseline.
//!
//! These bands catch large regressions on the development machine. They are
//! deliberately not hardware-performance claims or ABI promises (D-230).

use std::fs;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::cargo::Profile;
use crate::paths::Paths;
use crate::{image, paths, qemu, unsafety};

const RUNS: usize = 3;
const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Band {
    minimum: u64,
    maximum: u64,
}

impl Band {
    const fn new(minimum: u64, maximum: u64) -> Self {
        Self { minimum, maximum }
    }

    const fn exact(value: u64) -> Self {
        Self::new(value, value)
    }

    fn check(self, name: &str, value: u64) -> Result<()> {
        if value >= self.minimum && value <= self.maximum {
            Ok(())
        } else {
            bail!(
                "{name} measured {value}, outside {}..={}",
                self.minimum,
                self.maximum
            )
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Limits {
    kernel_bytes: Band,
    boot_ns: Band,
    frames_free: Band,
    heap_slab: Band,
    heap_large: Band,
    heap_live: Band,
    handles_committed: Band,
    handles_live: Band,
    trap_ns: Band,
    storage_bytes: Band,
    storage_bytes_per_second: Band,
    counter_hz: Band,
}

const RELEASE: Limits = Limits {
    kernel_bytes: Band::new(9_200_000, 10_100_000),
    boot_ns: Band::new(3_500_000, 9_000_000),
    frames_free: Band::new(114_000, 114_250),
    heap_slab: Band::new(16, 24),
    heap_large: Band::new(990, 1_010),
    heap_live: Band::exact(7),
    handles_committed: Band::exact(0),
    handles_live: Band::exact(0),
    trap_ns: Band::new(10_000, 100_000),
    storage_bytes: Band::exact(4 * KIB),
    storage_bytes_per_second: Band::new(10 * MIB, 100 * MIB),
    counter_hz: Band::new(3_000_000_000, 4_000_000_000),
};

const DEBUG: Limits = Limits {
    kernel_bytes: Band::new(12_100_000, 13_400_000),
    boot_ns: Band::new(20_000_000, 45_000_000),
    frames_free: Band::new(113_000, 113_300),
    heap_slab: Band::new(14, 24),
    heap_large: Band::new(990, 1_010),
    heap_live: Band::exact(7),
    handles_committed: Band::exact(0),
    handles_live: Band::exact(0),
    trap_ns: Band::new(50_000, 400_000),
    storage_bytes: Band::exact(4 * KIB),
    storage_bytes_per_second: Band::new(3 * MIB, 20 * MIB),
    counter_hz: Band::new(3_000_000_000, 4_000_000_000),
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Derived {
    trap_ns: u64,
    storage_bytes_per_second: u64,
}

/// Repeat both self-test profiles and enforce their reviewed bands.
pub fn check(paths: &Paths) -> Result<()> {
    let (unsafe_lines, unsafe_blocks) = unsafety::measure(&paths.root().join("wost"))?;
    Band::exact(unsafety::MEASURED as u64).check("unsafe lines", unsafe_lines as u64)?;
    println!("[xtask] baseline host: unsafe_lines={unsafe_lines} unsafe_blocks={unsafe_blocks}");

    run_profile(paths, Profile::Release, RELEASE, Duration::from_secs(60))?;
    run_profile(paths, Profile::Dev, DEBUG, Duration::from_secs(150))?;
    println!("[xtask] foundation baseline passed across {RUNS} release and {RUNS} debug boots");
    Ok(())
}

fn run_profile(paths: &Paths, profile: Profile, limits: Limits, timeout: Duration) -> Result<()> {
    let label = profile.directory();
    print_limits(label, limits);
    let image = image::build_for_run(paths, profile, image::Kernel::SelfTest)?;
    let kernel_bytes = fs::metadata(paths.kernel_elf(profile))
        .with_context(|| format!("reading {label} kernel size"))?
        .len();
    let image_bytes = fs::metadata(image.path())
        .with_context(|| format!("reading {label} image size"))?
        .len();
    limits
        .kernel_bytes
        .check(&format!("{label} kernel bytes"), kernel_bytes)?;
    Band::exact(paths::DISK_MIB * MIB).check(&format!("{label} image bytes"), image_bytes)?;
    println!("[xtask] baseline {label}: kernel_bytes={kernel_bytes} image_bytes={image_bytes}");

    let mut samples = Vec::with_capacity(RUNS);
    for run in 1..=RUNS {
        println!("[xtask] baseline {label}: boot {run}/{RUNS}");
        let sample = qemu::measure_performance(paths, image.path(), timeout)?;
        let derived = validate_sample(label, limits, sample)?;
        println!(
            "[xtask] baseline {label} sample {run}: boot_ns={} frames_free={} heap={}/{}/{} handles={}/{} trap_ns={} storage_bytes_per_second={}",
            sample.boot_ns,
            sample.frames_free,
            sample.heap_slab,
            sample.heap_large,
            sample.heap_live,
            sample.handles_committed,
            sample.handles_live,
            derived.trap_ns,
            derived.storage_bytes_per_second
        );
        samples.push((sample, derived));
    }

    print_ranges(label, &samples)
}

fn print_limits(label: &str, limits: Limits) {
    println!(
        "[xtask] baseline {label} bands: kernel_bytes={}..{} boot_ns={}..{} frames_free={}..{} heap_slab={}..{} heap_large={}..{} heap_live={}..{} handles_committed={}..{} handles_live={}..{} trap_ns={}..{} storage_bytes={}..{} storage_bytes_per_second={}..{} counter_hz={}..{}",
        limits.kernel_bytes.minimum,
        limits.kernel_bytes.maximum,
        limits.boot_ns.minimum,
        limits.boot_ns.maximum,
        limits.frames_free.minimum,
        limits.frames_free.maximum,
        limits.heap_slab.minimum,
        limits.heap_slab.maximum,
        limits.heap_large.minimum,
        limits.heap_large.maximum,
        limits.heap_live.minimum,
        limits.heap_live.maximum,
        limits.handles_committed.minimum,
        limits.handles_committed.maximum,
        limits.handles_live.minimum,
        limits.handles_live.maximum,
        limits.trap_ns.minimum,
        limits.trap_ns.maximum,
        limits.storage_bytes.minimum,
        limits.storage_bytes.maximum,
        limits.storage_bytes_per_second.minimum,
        limits.storage_bytes_per_second.maximum,
        limits.counter_hz.minimum,
        limits.counter_hz.maximum
    );
}

fn validate_sample(
    label: &str,
    limits: Limits,
    sample: qemu::PerformanceSample,
) -> Result<Derived> {
    let derived = Derived {
        trap_ns: ticks_to_units(sample.trap_ticks, 1_000_000_000, sample.counter_hz)
            .context("converting trap ticks to nanoseconds")?,
        storage_bytes_per_second: ticks_to_units(
            sample.storage_bytes,
            sample.counter_hz,
            sample.storage_ticks,
        )
        .context("converting storage ticks to bytes per second")?,
    };
    for (name, band, value) in [
        ("boot_ns", limits.boot_ns, sample.boot_ns),
        ("frames_free", limits.frames_free, sample.frames_free),
        ("heap_slab", limits.heap_slab, sample.heap_slab),
        ("heap_large", limits.heap_large, sample.heap_large),
        ("heap_live", limits.heap_live, sample.heap_live),
        (
            "handles_committed",
            limits.handles_committed,
            sample.handles_committed,
        ),
        ("handles_live", limits.handles_live, sample.handles_live),
        ("trap_ns", limits.trap_ns, derived.trap_ns),
        ("storage_bytes", limits.storage_bytes, sample.storage_bytes),
        (
            "storage_bytes_per_second",
            limits.storage_bytes_per_second,
            derived.storage_bytes_per_second,
        ),
        ("counter_hz", limits.counter_hz, sample.counter_hz),
    ] {
        band.check(&format!("{label} {name}"), value)?;
    }
    Ok(derived)
}

fn ticks_to_units(amount: u64, units: u64, ticks: u64) -> Option<u64> {
    let converted = u128::from(amount)
        .checked_mul(u128::from(units))?
        .checked_div(u128::from(ticks))?;
    u64::try_from(converted).ok()
}

fn observed_range(
    samples: &[(qemu::PerformanceSample, Derived)],
    value: impl Fn(&(qemu::PerformanceSample, Derived)) -> u64,
) -> Result<(u64, u64)> {
    let first = samples.first().context("no baseline samples")?;
    let mut minimum = value(first);
    let mut maximum = minimum;
    for sample in &samples[1..] {
        let current = value(sample);
        minimum = minimum.min(current);
        maximum = maximum.max(current);
    }
    Ok((minimum, maximum))
}

fn print_ranges(label: &str, samples: &[(qemu::PerformanceSample, Derived)]) -> Result<()> {
    let boot = observed_range(samples, |sample| sample.0.boot_ns)?;
    let frames = observed_range(samples, |sample| sample.0.frames_free)?;
    let heap_slab = observed_range(samples, |sample| sample.0.heap_slab)?;
    let heap_large = observed_range(samples, |sample| sample.0.heap_large)?;
    let heap_live = observed_range(samples, |sample| sample.0.heap_live)?;
    let committed = observed_range(samples, |sample| sample.0.handles_committed)?;
    let handles = observed_range(samples, |sample| sample.0.handles_live)?;
    let trap = observed_range(samples, |sample| sample.1.trap_ns)?;
    let storage = observed_range(samples, |sample| sample.1.storage_bytes_per_second)?;
    println!(
        "[xtask] baseline {label} ranges: boot_ns={}..{} frames_free={}..{} heap_slab={}..{} heap_large={}..{} heap_live={}..{} handles_committed={}..{} handles_live={}..{} trap_ns={}..{} storage_bytes_per_second={}..{}",
        boot.0,
        boot.1,
        frames.0,
        frames.1,
        heap_slab.0,
        heap_slab.1,
        heap_large.0,
        heap_large.1,
        heap_live.0,
        heap_live.1,
        committed.0,
        committed.1,
        handles.0,
        handles.1,
        trap.0,
        trap.1,
        storage.0,
        storage.1
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_math_and_bands_are_two_sided() {
        let band = Band::new(10, 20);
        assert!(band.check("fixture", 10).is_ok());
        assert!(band.check("fixture", 20).is_ok());
        assert!(band.check("fixture", 9).is_err());
        assert!(band.check("fixture", 21).is_err());

        assert_eq!(
            ticks_to_units(4_096, 2_000_000_000, 400_000),
            Some(20_480_000)
        );
        assert_eq!(ticks_to_units(u64::MAX, u64::MAX, 1), None);
        assert_eq!(ticks_to_units(1, 1, 0), None);
    }
}
