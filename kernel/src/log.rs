// Serial only, so diagnostics never scroll the banner away.

use core::fmt::{self, Write};

use crate::dev;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Level {
    #[cfg(feature = "selftest")]
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
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

const MAX_LEVEL: Level = Level::Info;

pub(crate) fn record(level: Level, args: fmt::Arguments<'_>) {
    if level > MAX_LEVEL {
        return;
    }
    dev::with_uart(|uart| {
        let _ = writeln!(uart, "[{}] {args}", level.tag());
    });
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        $crate::log::record($crate::log::Level::Error, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::log::record($crate::log::Level::Warn, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        $crate::log::record($crate::log::Level::Info, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        $crate::log::record($crate::log::Level::Debug, format_args!($($arg)*))
    };
}
