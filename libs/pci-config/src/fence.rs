use crate::{
    Address, DEVICES, FUNCTIONS, QualifiedNonRequester, QuiescedIsaRequester, command, header,
    is_absent,
};

/// One configuration-space operation in the entry-time DMA fence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HandoffAccess {
    /// Read the complete vendor/device identity dword.
    ReadIdentity,
    /// Read the complete revision/programming-interface/subclass/class dword.
    ReadClass,
    /// Read the command register; only its low sixteen bits are consumed.
    ReadCommand,
    /// Write command bits with a zero status half.
    WriteCommand(u16),
    /// Master-clear and mask every channel of both legacy 8237 controllers,
    /// then read their status registers: DMA1 in bits 0..8, DMA2 in 8..16.
    QuiesceIsaDma,
}

/// Request bits of both 8237 status registers in a [`HandoffAccess::QuiesceIsaDma`]
/// word; the terminal-count bits beside them clear on read and prove nothing.
pub const ISA_DMA_PENDING_REQUESTS: u32 = 0xf0f0;

/// Function authorities the platform PCI registry and kernel can hold at once.
///
/// A unit-wide VT-d fence claims every live requester the unit governs. The
/// default VMware machine alone presents 42, 32 of them root ports, so the
/// bound is sized by the qualified platform rather than by the test machine.
pub const FUNCTION_AUTHORITY_LIMIT: usize = 64;

/// Platform-bound policy for the entry-time DMA fence.
///
/// The VMware profile recognizes one exact emulated 440BX host bridge whose
/// command bit 2 is a read-only chipset control rather than a PCI requester
/// bus-master enable, and one exact PIIX4 ISA bridge whose read-only bit 2 does
/// govern a requester: its 8237 controllers. That requester stays in every
/// inventory and is quiesced at the controllers instead. The caller must select
/// the profile only from requester-inaccessible VMware CPU evidence; every
/// other function and every near miss remains under the strict BME-off rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HandoffFencePolicy {
    kind: HandoffFencePolicyKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HandoffFencePolicyKind {
    Strict,
    Vmware440Bx,
}

impl HandoffFencePolicy {
    /// Require every present PCI function to read back with BME clear.
    pub const fn strict() -> Self {
        Self {
            kind: HandoffFencePolicyKind::Strict,
        }
    }

    /// Admit only VMware's exact non-requesting 440BX host-bridge command.
    pub const fn vmware_440bx() -> Self {
        Self {
            kind: HandoffFencePolicyKind::Vmware440Bx,
        }
    }

    /// Classify the one exact platform function allowed to retain command bit 2.
    pub(crate) fn classify_non_requester(
        self,
        at: Address,
        identity: u32,
        class: u32,
        before: u16,
        found: u16,
    ) -> Option<QualifiedNonRequester> {
        const VMWARE_440BX_IDENTITY: u32 = 0x7190_8086;
        const HOST_BRIDGE_CLASS: u32 = 0x06_00_00_00;
        const CLASS_WITHOUT_REVISION: u32 = 0xffff_ff00;
        const VMWARE_HOST_COMMAND: u16 = command::MEMORY_SPACE | command::BUS_MASTER;

        (matches!(self.kind, HandoffFencePolicyKind::Vmware440Bx)
            && at.bus() == 0
            && at.device() == 0
            && at.function() == 0
            && identity == VMWARE_440BX_IDENTITY
            && class & CLASS_WITHOUT_REVISION == HOST_BRIDGE_CLASS
            && before == VMWARE_HOST_COMMAND
            && found == VMWARE_HOST_COMMAND)
            .then_some(QualifiedNonRequester {
                at: at.function_base(),
                identity,
                class,
                command: found,
            })
    }

    /// Whether this policy may quiesce an ISA requester at its 8237 controllers.
    pub const fn quiesces_isa_dma(self) -> bool {
        matches!(self.kind, HandoffFencePolicyKind::Vmware440Bx)
    }

    /// Whether `at` is VMware's exact PIIX4 ISA bridge with read-only BME.
    fn classify_isa_requester(
        self,
        at: Address,
        identity: u32,
        class: u32,
        before: u16,
        found: u16,
    ) -> bool {
        const VMWARE_PIIX4_ISA_IDENTITY: u32 = 0x7110_8086;
        const ISA_BRIDGE_CLASS: u32 = 0x06_01_00_00;
        const ISA_CLASS_MASK: u32 = 0xffff_ff00;
        const VMWARE_ISA_COMMAND: u16 =
            command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER;

        self.quiesces_isa_dma()
            && (at.bus(), at.device(), at.function()) == (0, 7, 0)
            && identity == VMWARE_PIIX4_ISA_IDENTITY
            && class & ISA_CLASS_MASK == ISA_BRIDGE_CLASS
            && before == VMWARE_ISA_COMMAND
            && found == VMWARE_ISA_COMMAND
    }
}

impl QualifiedNonRequester {
    /// Exact function whose entry-time fence minted this authority.
    pub const fn address(&self) -> Address {
        self.at
    }
}

/// What an exhaustive entry-time DMA fence observed.
#[derive(Debug)]
pub struct HandoffFenceReport {
    functions: u32,
    disabled: u32,
    retained_non_requesters: u32,
    retained_non_requester: Option<QualifiedNonRequester>,
    quiesced_requesters: u32,
    quiesced_requester: Option<QuiescedIsaRequester>,
}

impl HandoffFenceReport {
    /// Requesters held off at their 8237 controllers because BME is read-only.
    pub const fn quiesced_requesters(&self) -> u32 {
        self.quiesced_requesters
    }

    /// Remove the exact entry-quiesce authority for the later topology snapshot.
    pub fn take_quiesced_requester(&mut self) -> Option<QuiescedIsaRequester> {
        self.quiesced_requester.take()
    }

    /// Present functions safely classified by the platform-bound fence.
    pub const fn functions(&self) -> u32 {
        self.functions
    }

    /// Functions whose inherited BME bit the fence cleared.
    pub const fn disabled(&self) -> u32 {
        self.disabled
    }

    /// Qualified non-requester host bridges whose read-only bit 2 remained set.
    pub const fn retained_non_requesters(&self) -> u32 {
        self.retained_non_requesters
    }

    /// Remove the exact entry-fence authority for the later topology snapshot.
    pub fn take_non_requester(&mut self) -> Option<QualifiedNonRequester> {
        self.retained_non_requester.take()
    }
}

/// Why the entry-time requester fence could not prove DMA quiescence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum HandoffFenceError {
    /// One required configuration-space cycle was unavailable.
    Access {
        /// Function whose configuration cycle failed.
        at: Address,
        /// Operation that could not complete.
        operation: HandoffAccess,
    },
    /// BME clear did not preserve and read back the exact command word.
    CommandRefused {
        /// Function whose command word refused the fence.
        at: Address,
        /// Stable vendor/device identity observed around the transaction.
        identity: u32,
        /// Stable class dword observed around the transaction.
        class: u32,
        /// Exact BME-off word written or required.
        expected: u16,
        /// Word returned by the readback.
        found: u16,
    },
    /// The quiesced ISA requester still showed a DMA request after masking.
    IsaDmaPending {
        /// Function whose 8237 controllers were masked.
        at: Address,
        /// Combined status word: DMA1 in bits 0..8, DMA2 in 8..16.
        status: u32,
    },
    /// A present function changed identity during its fence.
    IdentityChanged {
        /// Function whose identity changed.
        at: Address,
        /// Identity read before the command transaction.
        expected: u32,
        /// Identity read after the command transaction.
        found: u32,
    },
    /// A present function changed class during its fence.
    ClassChanged {
        /// Function whose class changed.
        at: Address,
        /// Class dword read before the command transaction.
        expected: u32,
        /// Class dword read after the command transaction.
        found: u32,
    },
}

impl core::fmt::Display for HandoffFenceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Access { at, operation } => write!(
                f,
                "configuration {operation:?} failed for {:02x}:{:02x}.{}",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::CommandRefused {
                at,
                identity,
                class,
                expected,
                found,
            } => write!(
                f,
                "{:02x}:{:02x}.{} identity {identity:#010x} class {class:#010x} command read back as {found:#06x}, expected {expected:#06x}",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::IsaDmaPending { at, status } => write!(
                f,
                "{:02x}:{:02x}.{} 8237 status {status:#06x} still requests DMA after masking",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::IdentityChanged {
                at,
                expected,
                found,
            } => write!(
                f,
                "{:02x}:{:02x}.{} identity changed from {expected:#010x} to {found:#010x}",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::ClassChanged {
                at,
                expected,
                found,
            } => write!(
                f,
                "{:02x}:{:02x}.{} class changed from {expected:#010x} to {found:#010x}",
                at.bus(),
                at.device(),
                at.function()
            ),
        }
    }
}

impl core::error::Error for HandoffFenceError {}

/// Clear and verify BME on every present function in the legacy PCI segment.
///
/// Every bus, device and function number is probed independently; no header
/// hint or bridge route can shrink the inventory. The caller must already own
/// the one configuration mechanism used by `access` and hold trusted platform
/// authority that its visibility and requester identities remain static.
///
/// # Errors
///
/// Returns the exact unavailable cycle, refused command readback or changing
/// function identity. A refusal never restores BME on a function already
/// fenced.
pub fn fence_dma_handoff(
    policy: HandoffFencePolicy,
    mut access: impl FnMut(Address, HandoffAccess) -> Option<u32>,
) -> Result<HandoffFenceReport, HandoffFenceError> {
    let mut functions = 0u32;
    let mut disabled = 0u32;
    let mut retained_non_requesters = 0u32;
    let mut retained_non_requester = None;
    let mut quiesced_requesters = 0u32;
    let mut quiesced_requester = None;
    for bus in 0..=u8::MAX {
        for device in 0..DEVICES {
            for function in 0..FUNCTIONS {
                let Some(at) = Address::new(bus, device, function, header::VENDOR) else {
                    continue;
                };
                let identity =
                    access(at, HandoffAccess::ReadIdentity).ok_or(HandoffFenceError::Access {
                        at,
                        operation: HandoffAccess::ReadIdentity,
                    })?;
                if is_absent(identity as u16) {
                    continue;
                }
                let Some(command_at) = at.at(header::COMMAND) else {
                    continue;
                };
                let before = access(command_at, HandoffAccess::ReadCommand).ok_or(
                    HandoffFenceError::Access {
                        at,
                        operation: HandoffAccess::ReadCommand,
                    },
                )? as u16;
                let expected = before & !command::BUS_MASTER;
                if before != expected {
                    access(command_at, HandoffAccess::WriteCommand(expected)).ok_or(
                        HandoffFenceError::Access {
                            at,
                            operation: HandoffAccess::WriteCommand(expected),
                        },
                    )?;
                }
                let found = access(command_at, HandoffAccess::ReadCommand).ok_or(
                    HandoffFenceError::Access {
                        at,
                        operation: HandoffAccess::ReadCommand,
                    },
                )? as u16;
                let retained_class = if found != expected {
                    let Some(class_at) = at.at(header::CLASS) else {
                        continue;
                    };
                    let class = access(class_at, HandoffAccess::ReadClass).ok_or(
                        HandoffFenceError::Access {
                            at,
                            operation: HandoffAccess::ReadClass,
                        },
                    )?;
                    if let Some(authority) =
                        policy.classify_non_requester(at, identity, class, before, found)
                    {
                        Some((class_at, class, Retained::NonRequester(authority)))
                    } else if policy.classify_isa_requester(at, identity, class, before, found) {
                        let status = access(at, HandoffAccess::QuiesceIsaDma).ok_or(
                            HandoffFenceError::Access {
                                at,
                                operation: HandoffAccess::QuiesceIsaDma,
                            },
                        )?;
                        if status & ISA_DMA_PENDING_REQUESTS != 0 {
                            return Err(HandoffFenceError::IsaDmaPending { at, status });
                        }
                        let authority = QuiescedIsaRequester {
                            at: at.function_base(),
                            identity,
                            class,
                            command: found,
                        };
                        Some((class_at, class, Retained::Quiesced(authority)))
                    } else {
                        return Err(HandoffFenceError::CommandRefused {
                            at,
                            identity,
                            class,
                            expected,
                            found,
                        });
                    }
                } else if before != expected {
                    disabled += 1;
                    None
                } else {
                    None
                };
                let current =
                    access(at, HandoffAccess::ReadIdentity).ok_or(HandoffFenceError::Access {
                        at,
                        operation: HandoffAccess::ReadIdentity,
                    })?;
                if current != identity {
                    return Err(HandoffFenceError::IdentityChanged {
                        at,
                        expected: identity,
                        found: current,
                    });
                }
                if let Some((class_at, class, retained)) = retained_class {
                    let current_class = access(class_at, HandoffAccess::ReadClass).ok_or(
                        HandoffFenceError::Access {
                            at,
                            operation: HandoffAccess::ReadClass,
                        },
                    )?;
                    if current_class != class {
                        return Err(HandoffFenceError::ClassChanged {
                            at,
                            expected: class,
                            found: current_class,
                        });
                    }
                    match retained {
                        Retained::NonRequester(authority) => {
                            retained_non_requesters += 1;
                            retained_non_requester = Some(authority);
                        }
                        Retained::Quiesced(authority) => {
                            quiesced_requesters += 1;
                            quiesced_requester = Some(authority);
                        }
                    }
                }
                functions += 1;
            }
        }
    }
    Ok(HandoffFenceReport {
        functions,
        disabled,
        retained_non_requesters,
        retained_non_requester,
        quiesced_requesters,
        quiesced_requester,
    })
}

/// The exact authority a refused BME clear can still mint at entry.
enum Retained {
    NonRequester(QualifiedNonRequester),
    Quiesced(QuiescedIsaRequester),
}
