//! Kernel logging.
//!
//! Records go to the serial console only. The framebuffer carries user-facing
//! output, so diagnostics never scroll the banner away.

use core::fmt::{self, Write};

use crate::dev;

/// Severity of a log record, most severe first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Level {
    #[cfg(feature = "selftest")]
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
    /// Fixed-width tag, so records line up in the serial log.
    const fn tag(self) -> &'static str {
        match self {
            #[cfg(feature = "selftest")]
            Self::Error => "error",
            Self::Warn => "warn ",
            Self::Info => "info ",
            Self::Debug => "debug",
        }
    }
}

/// Records less severe than this are discarded at the call site.
const MAX_LEVEL: Level = Level::Info;

/// Emit one record. Use the [`error!`](crate::error), [`warn!`](crate::warn),
/// [`info!`](crate::info), and [`debug!`](crate::debug) macros instead.
pub(crate) fn record(level: Level, args: fmt::Arguments<'_>) {
    if level > MAX_LEVEL {
        return;
    }
    dev::with_uart(|uart| {
        let _ = writeln!(uart, "[{}] {args}", level.tag());
    });
}

/// Log a failure the kernel could not recover from cleanly.
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        $crate::log::record($crate::log::Level::Error, format_args!($($arg)*))
    };
}

/// Log a condition that is survivable but not expected.
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::log::record($crate::log::Level::Warn, format_args!($($arg)*))
    };
}

/// Log a milestone in normal operation.
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        $crate::log::record($crate::log::Level::Info, format_args!($($arg)*))
    };
}

/// Log detail useful only when chasing a specific problem.
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        $crate::log::record($crate::log::Level::Debug, format_args!($($arg)*))
    };
}
