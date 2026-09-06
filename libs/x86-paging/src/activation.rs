//! Paging-activation publication states.

/// One phase of the address-space activation transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    /// No activation owns the processor's page-table transition.
    Cold = 0,
    /// One activation owns the unpublished transition.
    Initializing = 1,
    /// The installed root and readiness are both published.
    Ready = 2,
}

impl State {
    /// Decode an atomically published state.
    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits {
            0 => Some(Self::Cold),
            1 => Some(Self::Initializing),
            2 => Some(Self::Ready),
            _ => None,
        }
    }

    /// Encode this state for atomic publication.
    pub const fn bits(self) -> u8 {
        self as u8
    }

    /// Begin the transaction if no activation owns it.
    pub const fn begin(self) -> Option<Self> {
        match self {
            Self::Cold => Some(Self::Initializing),
            Self::Initializing | Self::Ready => None,
        }
    }

    /// Publish readiness after initialization has finished.
    pub const fn complete(self) -> Option<Self> {
        match self {
            Self::Initializing => Some(Self::Ready),
            Self::Cold | Self::Ready => None,
        }
    }

    /// Return an uncommitted initialization to the cold state.
    pub const fn abort(self) -> Option<Self> {
        match self {
            Self::Initializing => Some(Self::Cold),
            Self::Cold | Self::Ready => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::State;

    #[test]
    fn every_byte_decodes_to_one_state_or_refusal() {
        for bits in u8::MIN..=u8::MAX {
            let decoded = State::from_bits(bits);
            if bits <= State::Ready.bits() {
                assert_eq!(decoded.map(State::bits), Some(bits));
            } else {
                assert_eq!(decoded, None, "unexpected state byte {bits}");
            }
        }
    }

    #[test]
    fn only_the_protocol_edges_are_admitted() {
        assert_eq!(State::Cold.begin(), Some(State::Initializing));
        assert_eq!(State::Initializing.begin(), None);
        assert_eq!(State::Ready.begin(), None);

        assert_eq!(State::Cold.complete(), None);
        assert_eq!(State::Initializing.complete(), Some(State::Ready));
        assert_eq!(State::Ready.complete(), None);

        assert_eq!(State::Cold.abort(), None);
        assert_eq!(State::Initializing.abort(), Some(State::Cold));
        assert_eq!(State::Ready.abort(), None);
    }
}
