use core::sync::atomic::{AtomicBool, Ordering};

use super::{
    Controller, FLUSH_LIMIT, I8042Error, RESPONSE_ATTEMPTS, command, config, device, drains, reply,
};

/// An MF2 keyboard's identity when the controller does not rewrite its reply.
#[cfg(feature = "selftest")]
pub(crate) const UNTRANSLATED_MF2: (u8, u8) = (0xAB, 0x83);

/// Controller-side scancode translation requested by a self-test transaction.
#[cfg(feature = "selftest")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Translation {
    Enabled,
    Disabled,
}

/// Selection argument that reads the current set back instead of setting one.
#[cfg(feature = "selftest")]
const CURRENT_SET: u8 = 0x00;

#[cfg(feature = "selftest")]
pub(super) static REFUSE_CONFIG_WRITE: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(super) static REFUSE_FLUSH: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(super) static REFUSE_CONTROL_COMMAND: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(super) static CONTROL_COMMAND_REFUSED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(super) static REFUSE_INTERRUPT_READBACK: AtomicBool = AtomicBool::new(false);

impl Controller {
    /// Refuse the next post-write interrupt-byte readback.
    #[cfg(feature = "selftest")]
    pub(crate) fn refuse_interrupt_readback() {
        REFUSE_INTERRUPT_READBACK.store(true, Ordering::Relaxed);
    }

    /// The configuration byte, read live from the controller.
    #[cfg(feature = "selftest")]
    pub(crate) fn config(&mut self) -> Option<u8> {
        if !Self::flush(self) {
            return None;
        }
        self.read_config()
    }

    /// Send the echo command and report whether the echo came back.
    #[cfg(feature = "selftest")]
    pub(crate) fn echo_roundtrips(&mut self) -> bool {
        if !Self::flush(self) {
            return false;
        }
        self.write_data(device::ECHO) && self.read_reply(RESPONSE_ATTEMPTS) == Some(reply::ECHO)
    }

    /// The scancode set the keyboard reports itself speaking.
    #[cfg(feature = "selftest")]
    pub(crate) fn scancode_set(&mut self) -> Option<u8> {
        if !Self::flush(self) {
            return None;
        }
        if !self.device_command(device::SET_SCANCODE_SET) || !self.device_command(CURRENT_SET) {
            return None;
        }
        self.read_reply(RESPONSE_ATTEMPTS)
    }

    /// Turn the controller's translation on or off, and report what stuck.
    /// The self-test's only way to reach set 1 with a known key: QEMU honours this bit, so it
    /// reproduces what VMware does unasked, with an injected keystroke the harness chose.
    #[cfg(feature = "selftest")]
    pub(crate) fn set_translation(&mut self, translation: Translation) -> bool {
        if !Self::flush(self) {
            return false;
        }
        let Some(current) = self.read_config() else {
            return false;
        };
        let wanted = match translation {
            Translation::Enabled => current | config::TRANSLATE,
            Translation::Disabled => current & !config::TRANSLATE,
        };
        self.write_config(wanted)
    }

    /// Whether a required configuration refusal is typed and non-mutating.
    #[cfg(feature = "selftest")]
    pub(crate) fn configuration_refusal_is_visible(&mut self) -> bool {
        let Some(current) = self.config() else {
            return false;
        };
        REFUSE_CONFIG_WRITE.store(true, Ordering::Relaxed);
        let refused = self.require_config(current) == Err(I8042Error::ConfigurationRefused);
        let restored = !REFUSE_CONFIG_WRITE.swap(false, Ordering::Relaxed);
        refused && restored && self.config() == Some(current)
    }

    /// Whether every transaction stops at a refused output-buffer flush.
    #[cfg(feature = "selftest")]
    pub(crate) fn flush_refusals_are_visible(&mut self) -> bool {
        let Some(initial) = self.config() else {
            return false;
        };
        let translation = if initial & config::TRANSLATE != 0 {
            Translation::Enabled
        } else {
            Translation::Disabled
        };
        let prepared = self.enable_interrupt().is_ok()
            && self.config().is_some()
            && self.echo_roundtrips()
            && self.scancode_set().is_some()
            && self.identity().is_some()
            && self.set_translation(translation);
        let expected = if prepared { self.config() } else { None };

        let (interrupt, interrupt_consumed) = self.with_refused_flush(Self::enable_interrupt);
        let (config, config_consumed) = self.with_refused_flush(Self::config);
        let (echo, echo_consumed) = self.with_refused_flush(Self::echo_roundtrips);
        let (scancode, scancode_consumed) = self.with_refused_flush(Self::scancode_set);
        let (identity, identity_consumed) = self.with_refused_flush(Self::identity);
        let (translation, translation_consumed) =
            self.with_refused_flush(|controller| controller.set_translation(translation));

        let refused = prepared
            && expected.is_some()
            && interrupt.is_err()
            && interrupt_consumed
            && config.is_none()
            && config_consumed
            && !echo
            && echo_consumed
            && scancode.is_none()
            && scancode_consumed
            && identity.is_none()
            && identity_consumed
            && !translation
            && translation_consumed
            && self.config() == expected;
        let restored = self.write_config(initial) && self.config() == Some(initial);
        refused && restored
    }

    /// Whether a refused control command is typed and consumes no reply.
    #[cfg(feature = "selftest")]
    pub(crate) fn transport_refusals_are_visible(&mut self) -> bool {
        let Some(expected) = self.config() else {
            return false;
        };

        REFUSE_CONTROL_COMMAND.store(true, Ordering::Relaxed);
        let typed = self.require_controller_query(command::READ_CONFIG, I8042Error::NotPresent)
            == Err(I8042Error::TransportRefused);
        let typed_consumed = !REFUSE_CONTROL_COMMAND.swap(false, Ordering::Relaxed);
        let typed_did_not_read = CONTROL_COMMAND_REFUSED.swap(false, Ordering::Relaxed);

        REFUSE_CONTROL_COMMAND.store(true, Ordering::Relaxed);
        let query_refused = self.read_config().is_none();
        let query_consumed = !REFUSE_CONTROL_COMMAND.swap(false, Ordering::Relaxed);
        let query_did_not_read = CONTROL_COMMAND_REFUSED.swap(false, Ordering::Relaxed);

        typed
            && typed_consumed
            && typed_did_not_read
            && query_refused
            && query_consumed
            && query_did_not_read
            && self.config() == Some(expected)
    }

    #[cfg(feature = "selftest")]
    fn with_refused_flush<T>(&mut self, operation: impl FnOnce(&mut Self) -> T) -> (T, bool) {
        REFUSE_FLUSH.store(true, Ordering::Relaxed);
        let outcome = operation(self);
        let consumed = !REFUSE_FLUSH.swap(false, Ordering::Relaxed);
        (outcome, consumed)
    }
}

/// Whether a backlog of exactly the flush bound drains and one more does not.
#[cfg(feature = "selftest")]
pub(crate) fn a_backlog_of_the_flush_bound_drains_for_test() -> bool {
    let drains_backlog = |backlog: u32| {
        let waiting = core::cell::Cell::new(backlog);
        drains(
            FLUSH_LIMIT,
            || waiting.get() != 0,
            || waiting.set(waiting.get() - 1),
        )
    };
    drains_backlog(FLUSH_LIMIT) && !drains_backlog(FLUSH_LIMIT + 1)
}
