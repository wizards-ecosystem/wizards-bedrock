#[cfg(feature = "selftest")]
use core::sync::atomic::Ordering;

use nvme::{
    Capabilities, Capacity, Command, Completion, NAMESPACE_IDENTIFY_BYTES, Namespace, PollLimit,
    ReadTransfer,
};
#[cfg(feature = "selftest")]
use nvme::{NamespaceError, TransferError};
#[cfg(feature = "selftest")]
use wost::arch::current::cpu;
use wost::arch::current::vtd::Vtd;
use wost::mm::{self, DmaSegment};
use x86_paging::PAGE_SIZE;

#[cfg(feature = "selftest")]
use super::selftest::{COMMAND_HELD, HOLD_COMMAND};
use super::{
    BringUpError, COMMAND_TIMEOUT_MS, Controller, ControllerRetirement, IO_QUEUE, NAMESPACE,
    POLL_ATTEMPTS, QueuePair, clock_ns, quarantine_controller, stop_controller, write,
    write_submission,
};
use crate::thread::ThreadContext;
use crate::{mem, warn};

impl Controller {
    /// Commands this controller has been given, counted only after the doorbell write succeeds,
    /// so a local refusal cannot satisfy evidence that the device was asked.
    #[cfg(feature = "selftest")]
    pub(crate) const fn commands(&self) -> u32 {
        self.published_commands
    }

    /// Where the register window is readable.
    #[cfg(feature = "selftest")]
    pub(crate) const fn window(&self) -> u64 {
        self.observed_window
    }

    /// What `CAP` said.
    pub(crate) const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Entries in each queue.
    pub(crate) const fn queue_entries(&self) -> u32 {
        self.admin.capacity.entries()
    }

    /// Whether an I/O queue pair was built.
    #[cfg(feature = "selftest")]
    pub(crate) const fn has_io_queue(&self) -> bool {
        self.io.is_some()
    }

    /// Ask the namespace how large it is and how large its blocks are.
    pub(super) fn identify_namespace(&mut self, capacity: Capacity) -> Result<(), BringUpError> {
        let mut buffer =
            DmaSegment::from(mm::allocate_contiguous(1, 1).ok_or(BringUpError::NoMemory)?);
        if !buffer.zero() {
            return Err(BringUpError::NoMemory);
        }
        let address = buffer.address().ok_or(BringUpError::NoMemory)?;

        let command = Command::identify_namespace(self.take_id(), NAMESPACE, address, PAGE_SIZE)
            .ok_or(BringUpError::NotReady)?;
        self.submit_admin(command, Some(&mut buffer))?;

        let mut identify = [0u8; NAMESPACE_IDENTIFY_BYTES];
        if !buffer.read_at(0, &mut identify) {
            return Err(BringUpError::NoMemory);
        }
        self.configure_namespace(&identify, capacity)
    }

    /// Validate and commit namespace geometry before I/O queues can exist.
    fn configure_namespace(
        &mut self,
        identify: &[u8; NAMESPACE_IDENTIFY_BYTES],
        capacity: Capacity,
    ) -> Result<(), BringUpError> {
        let namespace = Namespace::parse(identify).map_err(BringUpError::InvalidNamespace)?;
        self.create_io_queues(capacity)?;
        self.blocks = namespace.blocks();
        self.block_bytes = namespace.block_bytes();
        Ok(())
    }

    /// Build the one I/O queue pair, with its interrupts disabled.
    fn create_io_queues(&mut self, capacity: Capacity) -> Result<(), BringUpError> {
        #[cfg(feature = "selftest")]
        if self.intercept_io_queue_creation {
            self.intercepted_io_queue_creations += 1;
            return Err(BringUpError::NotReady);
        }

        let negotiation =
            Command::number_of_queues(self.take_id(), 1, 1).ok_or(BringUpError::QueueSize)?;
        self.submit_admin(negotiation, None)?;

        let pair = QueuePair::new(IO_QUEUE, capacity).ok_or(BringUpError::NoMemory)?;
        // Owned before the first fallible domain transition: retirement must see an uncertain
        // partial map, and keeps device and domain quarantined until the leaves are invalidated.
        self.io = Some(pair);
        // Reachable before the controller can be told about either queue.
        {
            let mut installed = crate::iommu::installed();
            let isolation = installed.as_mut().ok_or(BringUpError::Isolation)?;
            let pair = self.io.as_mut().ok_or(BringUpError::Isolation)?;
            if !pair.map_into(isolation) {
                return Err(BringUpError::Isolation);
            }
        }
        let pair = self.io.as_ref().ok_or(BringUpError::Isolation)?;
        let completion_address = pair.completion.address().ok_or(BringUpError::NoMemory)?;
        let submission_address = pair.submission.address().ok_or(BringUpError::NoMemory)?;
        // Once either command is published the controller can retain these runs, so cleanup owns
        // both. Completion first since submission names it; clear `IEN` keeps it polled (`012` §2).
        let completion = Command::create_completion_queue(
            self.take_id(),
            IO_QUEUE,
            capacity,
            completion_address,
            PAGE_SIZE,
        )
        .ok_or(BringUpError::QueueSize)?;
        self.submit_admin(completion, None)?;

        let submission = Command::create_submission_queue(
            self.take_id(),
            IO_QUEUE,
            IO_QUEUE,
            capacity,
            submission_address,
            PAGE_SIZE,
        )
        .ok_or(BringUpError::QueueSize)?;
        self.submit_admin(submission, None)?;

        Ok(())
    }

    /// Read `blocks` logical blocks starting at `lba` into `into`; a request that does not fit
    /// the command, namespace or DMA extent fails with `InvalidTransfer` before submission.
    pub(crate) fn read_blocks(
        &mut self,
        lba: u64,
        blocks: u32,
        into: &mut DmaSegment,
    ) -> Result<(), BringUpError> {
        let buffer_bytes = into.len().ok_or(BringUpError::NotReady)?;
        let request = self.validate_read(lba, blocks, buffer_bytes)?;
        self.submit_read(request, into)
    }

    /// Measure one read which fills the retained DMA buffer.
    #[cfg(feature = "selftest")]
    pub(crate) fn measure_buffered_read(&mut self, into: &mut DmaSegment) -> Option<(u64, u64)> {
        let buffer_bytes = into.len()?;
        let block_bytes = u64::from(self.block_bytes);
        if block_bytes == 0 {
            return None;
        }
        let blocks = u32::try_from(
            (buffer_bytes / block_bytes)
                .min(self.blocks)
                .min(u64::from(nvme::MAX_TRANSFER_BLOCKS)),
        )
        .ok()?;
        if blocks == 0 {
            return None;
        }
        let bytes = u64::from(blocks).checked_mul(block_bytes)?;
        let started = cpu::timestamp();
        self.read_blocks(0, blocks, into).ok()?;
        let ticks = cpu::timestamp().checked_sub(started)?;
        (ticks > 0).then_some((bytes, ticks))
    }

    /// Bind raw caller arguments to this namespace before submission.
    fn validate_read(
        &self,
        lba: u64,
        blocks: u32,
        buffer_bytes: u64,
    ) -> Result<ReadTransfer, BringUpError> {
        ReadTransfer::new(
            lba,
            blocks,
            self.blocks,
            u64::from(self.block_bytes),
            buffer_bytes,
        )
        .and_then(|request| request.within_limit(PAGE_SIZE))
        .map_err(BringUpError::InvalidTransfer)
    }

    /// Build and publish a read already proved to fit every outer bound.
    fn submit_read(
        &mut self,
        request: ReadTransfer,
        into: &mut DmaSegment,
    ) -> Result<(), BringUpError> {
        #[cfg(feature = "selftest")]
        if self.intercept_read_submission {
            self.intercepted_read_submissions += 1;
            return Err(BringUpError::NotReady);
        }

        if self.io.is_none() {
            return Err(BringUpError::NotReady);
        }
        let address = into.address().ok_or(BringUpError::NotReady)?;
        let command = Command::read(self.take_id(), NAMESPACE, address, PAGE_SIZE, request)
            .ok_or(BringUpError::NotReady)?;

        self.submit_on(IO_QUEUE, command, Some(into)).map(|_| ())
    }

    /// Prove malformed Identify data cannot reach I/O queue creation.
    #[cfg(feature = "selftest")]
    pub(crate) fn malformed_namespace_stops_before_io_queues(&mut self) -> bool {
        const SIZE: usize = 0;
        const FORMAT_COUNT: usize = 25;
        const FORMATTED: usize = 26;
        const PROTECTION: usize = 29;
        const FIRST_FORMAT: usize = 128;
        const METADATA: usize = FIRST_FORMAT;
        const BLOCK_SHIFT: usize = FIRST_FORMAT + 2;
        const FORMAT_CONTROL: usize = FIRST_FORMAT + 3;

        let capacity = self.admin.capacity;
        let blocks_before = self.blocks;
        let block_bytes_before = self.block_bytes;
        let commands_before = self.commands();
        let queue_before = self.has_io_queue();
        let intercepted_before = self.intercepted_io_queue_creations;

        let mut valid = [0u8; NAMESPACE_IDENTIFY_BYTES];
        valid[SIZE] = 1;
        valid[BLOCK_SHIFT] = nvme::SUPPORTED_BLOCK_BYTES.trailing_zeros() as u8;

        self.intercept_io_queue_creation = true;

        let mut empty = valid;
        empty[SIZE] = 0;
        let empty = self.configure_namespace(&empty, capacity);

        let mut formats = valid;
        formats[FORMAT_COUNT] = 16;
        let formats = self.configure_namespace(&formats, capacity);

        let mut outside = valid;
        outside[FORMATTED] = 1;
        let outside = self.configure_namespace(&outside, capacity);

        let mut reserved_selector = valid;
        reserved_selector[FORMATTED] = 0x80;
        let reserved_selector = self.configure_namespace(&reserved_selector, capacity);

        let mut reserved_format = valid;
        reserved_format[FORMAT_CONTROL] = 4;
        let reserved_format = self.configure_namespace(&reserved_format, capacity);

        let mut metadata = valid;
        metadata[METADATA] = 1;
        let metadata = self.configure_namespace(&metadata, capacity);

        let mut protection = valid;
        protection[PROTECTION] = 1;
        let protection = self.configure_namespace(&protection, capacity);

        let mut block_size = valid;
        block_size[BLOCK_SHIFT] = 32;
        let block_size = self.configure_namespace(&block_size, capacity);

        self.intercept_io_queue_creation = false;

        let refused = [
            (empty, NamespaceError::Empty),
            (formats, NamespaceError::TooManyFormats(17)),
            (
                outside,
                NamespaceError::FormatOutsideList {
                    selected: 1,
                    formats: 1,
                },
            ),
            (
                reserved_selector,
                NamespaceError::ReservedFormatBits(0x80),
            ),
            (
                reserved_format,
                NamespaceError::ReservedLbaFormatBits(4),
            ),
            (metadata, NamespaceError::Metadata(1)),
            (protection, NamespaceError::Protection(1)),
            (block_size, NamespaceError::BlockSize(32)),
        ]
        .into_iter()
        .all(|(result, expected)| {
            matches!(result, Err(BringUpError::InvalidNamespace(found)) if found == expected)
        });

        refused
            && self.blocks == blocks_before
            && self.block_bytes == block_bytes_before
            && self.commands() == commands_before
            && self.has_io_queue() == queue_before
            && self.intercepted_io_queue_creations == intercepted_before
    }

    /// Prove malformed reads stop before the private submission seam.
    #[cfg(feature = "selftest")]
    pub(crate) fn malformed_reads_stop_before_submission(&mut self, into: &mut DmaSegment) -> bool {
        let Some(buffer_bytes) = into.len() else {
            return false;
        };
        let block_bytes = u64::from(self.block_bytes);
        if block_bytes == 0 {
            return false;
        }
        let short_blocks = buffer_bytes / block_bytes + 1;
        let Ok(short_blocks) = u32::try_from(short_blocks) else {
            return false;
        };
        if short_blocks > nvme::MAX_TRANSFER_BLOCKS || u64::from(short_blocks) > self.blocks {
            return false;
        }

        let commands_before = self.commands();
        let intercepted_before = self.intercepted_read_submissions;
        self.intercept_read_submission = true;
        let refused = [
            (
                self.read_blocks(0, 0, into),
                TransferError::Empty,
            ),
            (
                self.read_blocks(0, nvme::MAX_TRANSFER_BLOCKS + 1, into),
                TransferError::TooManyBlocks,
            ),
            (
                self.read_blocks(u64::MAX, 1, into),
                TransferError::OutsideNamespace,
            ),
            (
                self.read_blocks(self.blocks, 1, into),
                TransferError::OutsideNamespace,
            ),
            (
                self.read_blocks(0, short_blocks, into),
                TransferError::BufferTooSmall,
            ),
        ]
        .into_iter()
        .all(|(result, expected)| {
            matches!(result, Err(BringUpError::InvalidTransfer(found)) if found == expected)
        });
        self.intercept_read_submission = false;

        refused
            && self.commands() == commands_before
            && self.intercepted_read_submissions == intercepted_before
    }

    /// The next command identifier.
    fn take_id(&mut self) -> u16 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    /// Run one command on the admin queue.
    fn submit_admin(
        &mut self,
        command: Command,
        data: Option<&mut DmaSegment>,
    ) -> Result<Completion, BringUpError> {
        self.submit_on(0, command, data)
    }

    /// Submit `command` to queue `which` and spin until it is answered.
    /// Nothing is published without an installed DMA domain; the transfer buffer enters it
    /// before the doorbell and leaves once the completion proves the device is done with it,
    /// so the device reaches exactly the published and unretired set.
    fn submit_on(
        &mut self,
        which: u16,
        command: Command,
        mut data: Option<&mut DmaSegment>,
    ) -> Result<Completion, BringUpError> {
        let expected = command.id();
        let capabilities = self.capabilities;
        let submission_doorbell = nvme::doorbell(capabilities, which, nvme::Doorbell::Submission)
            .ok_or(BringUpError::NotReady)?;

        {
            let queue = self.queue_mut(which).ok_or(BringUpError::NotReady)?;
            if queue.capacity.has_room(queue.submission_head, queue.tail) != Some(true) {
                return Err(BringUpError::NotReady);
            }
        }
        if self.registers.is_none() {
            return Err(BringUpError::NotMapped);
        }
        let mut installed = crate::iommu::installed();
        let isolation = installed.as_mut().ok_or(BringUpError::Isolation)?;
        #[cfg(feature = "selftest")]
        let mapping = if self.leave_data_unmapped {
            DataMapping::Unmapped
        } else {
            DataMapping::Mapped
        };
        #[cfg(not(feature = "selftest"))]
        let mapping = DataMapping::Mapped;
        if mapping == DataMapping::Mapped
            && let Some(buffer) = data.as_deref_mut()
            && isolation.map_segment_for_device_write(buffer).is_err()
        {
            return Err(BringUpError::Isolation);
        }
        if !self.dma.reserve() {
            let _unmapped = unmap_data(isolation, data.as_deref_mut(), mapping);
            return Err(BringUpError::NotReady);
        }

        // Reserve ownership first: a refused staging cancels it, and only a rung doorbell needs
        // controller quiescence to recover.
        let submission_tail = match self.stage_submission(which, &command) {
            Ok(tail) => tail,
            Err(reason) => {
                if !self.dma.cancel_unpublished() {
                    panic!("an unpublished DMA reservation must be cancellable");
                }
                let _unmapped = unmap_data(isolation, data.as_deref_mut(), mapping);
                return Err(reason);
            }
        };
        let registers = &mut self.registers;
        let Some(rung) = self.dma.publish_with(|publication| {
            registers
                .as_mut()
                .ok_or(BringUpError::NotMapped)
                .and_then(|registers| {
                    write_submission(publication, registers, submission_doorbell, submission_tail)
                })
        }) else {
            panic!("a prepared DMA reservation must publish exactly once");
        };
        if let Err(error) = rung {
            return self.fail_submitted(error, data.as_deref_mut(), isolation, mapping);
        }
        #[cfg(feature = "selftest")]
        {
            self.published_commands = self.published_commands.saturating_add(1);
        }

        #[cfg(feature = "selftest")]
        if HOLD_COMMAND.load(Ordering::Acquire) {
            COMMAND_HELD.store(true, Ordering::Release);
            while HOLD_COMMAND.load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
            COMMAND_HELD.store(false, Ordering::Release);
        }

        let mut limit = PollLimit::after_ms(clock_ns(), COMMAND_TIMEOUT_MS, POLL_ATTEMPTS);
        #[cfg(feature = "selftest")]
        if self.force_timeout {
            limit = PollLimit::Attempts(0);
        }
        while limit.permit(clock_ns()) {
            let found = match self.poll_completion(which) {
                Ok(found) => found,
                Err(reason) => {
                    return self.fail_submitted(reason, data.as_deref_mut(), isolation, mapping);
                }
            };

            if let Some(entry) = found {
                if let Err(reason) = self.accept_completion(which, expected, &entry) {
                    return self.fail_submitted(reason, data.as_deref_mut(), isolation, mapping);
                }
                if !self.dma.complete() {
                    panic!("a matching completion must finish one published command");
                }
                // The completion proves the device is done, so the buffer leaves the domain first.
                if !unmap_data(isolation, data.as_deref_mut(), mapping) {
                    return self.fail_controller(BringUpError::Isolation);
                }
                if let Err(error) = self.acknowledge_completion(which) {
                    return self.fail_controller(error);
                }
                if !entry.is_success() {
                    return Err(BringUpError::Command(entry.status));
                }
                return Ok(entry);
            }
            core::hint::spin_loop();
        }
        self.fail_submitted(BringUpError::NotReady, data, isolation, mapping)
    }

    /// Write `command` at the tail of queue `which` and return the tail the doorbell publishes.
    fn stage_submission(&mut self, which: u16, command: &Command) -> Result<u32, BringUpError> {
        let queue = self.queue_mut(which).ok_or(BringUpError::NotReady)?;
        let at = queue.tail as usize * nvme::SUBMISSION_ENTRY_BYTES;
        if !queue.submission.write_at(at, &command.to_bytes()) {
            return Err(BringUpError::NoMemory);
        }
        queue.tail = Capacity::next(queue.capacity, queue.tail).ok_or(BringUpError::NotReady)?;
        Ok(queue.tail)
    }

    /// The entry at the completion head, if its phase tag says it is new (`012` §3).
    fn poll_completion(&mut self, which: u16) -> Result<Option<Completion>, BringUpError> {
        let queue = self.queue_mut(which).ok_or(BringUpError::NotReady)?;
        let at = queue.completion_head as usize * nvme::COMPLETION_ENTRY_BYTES;
        let mut bytes = [0u8; nvme::COMPLETION_ENTRY_BYTES];
        if !queue.completion.read_at(at, &mut bytes) {
            return Err(BringUpError::NoMemory);
        }
        if Completion::from_bytes(bytes).phase != queue.phase {
            return Ok(None);
        }
        DmaSegment::observe_device();
        if !queue.completion.read_at(at, &mut bytes) {
            return Err(BringUpError::NoMemory);
        }
        Ok(Some(Completion::from_bytes(bytes)))
    }

    /// Require `entry` to answer the command just published, then advance both heads.
    fn accept_completion(
        &mut self,
        which: u16,
        expected: u16,
        entry: &Completion,
    ) -> Result<(), BringUpError> {
        let provenance = self.queue_mut(which).is_some_and(|queue| {
            entry.phase == queue.phase
                && entry.matches_submission(which, expected, queue.tail, queue.capacity)
        });
        if !provenance {
            return Err(BringUpError::UnexpectedCompletion {
                queue: entry.submission_queue,
                id: entry.id,
                submission_head: entry.submission_head,
            });
        }
        let queue = self.queue_mut(which).ok_or(BringUpError::NotReady)?;
        queue.submission_head = u32::from(entry.submission_head);
        let completion_head =
            Capacity::next(queue.capacity, queue.completion_head).ok_or(BringUpError::NotReady)?;
        if completion_head == 0 {
            queue.phase = !queue.phase;
        }
        queue.completion_head = completion_head;
        Ok(())
    }

    /// Ring the completion doorbell with the new head.
    fn acknowledge_completion(&mut self, which: u16) -> Result<(), BringUpError> {
        let doorbell = nvme::doorbell(self.capabilities, which, nvme::Doorbell::Completion)
            .ok_or(BringUpError::NotReady)?;
        let head = self
            .queue_mut(which)
            .map(|queue| queue.completion_head)
            .ok_or(BringUpError::NotReady)?;
        self.registers
            .as_mut()
            .ok_or(BringUpError::NotMapped)
            .and_then(|registers| write(registers, doorbell, head))
    }

    /// Stop uncertain DMA before exposing an error to the caller. A buffer the stop proved
    /// quiescent leaves the domain; a retained one stays mapped, as the device may still reach it.
    fn fail_submitted<T>(
        &mut self,
        error: BringUpError,
        mut data: Option<&mut DmaSegment>,
        isolation: &mut Vtd,
        mapping: DataMapping,
    ) -> Result<T, BringUpError> {
        self.stop_dma(data.as_deref_mut());
        if !self.dma.must_retain() && !unmap_data(isolation, data, mapping) {
            warn!("nvme: a quiesced transfer buffer stays mapped in the dma domain");
        }
        Err(error)
    }

    /// Fail closed after a completion made the transfer buffer safe.
    fn fail_controller<T>(&mut self, error: BringUpError) -> Result<T, BringUpError> {
        self.stop_dma(None);
        Err(error)
    }

    /// Disable the controller and verify it idle, or withhold all DMA memory.
    pub(super) fn stop_dma(&mut self, data: Option<&mut DmaSegment>) {
        if !self.dma.is_settled() {
            match stop_controller(self) {
                Ok(stopped) => stopped.quiesce(),
                Err(_) => self.dma.retain_unquiesced(),
            }
        }

        if self.dma.must_retain() {
            self.admin.retain();
            if let Some(queue) = self.io.as_mut() {
                queue.retain();
            }
            if let Some(segment) = data {
                segment.retain();
            }
        }
    }

    /// Remove every queue from the installed domain before its frames drop.
    fn unmap_queues(&mut self) -> bool {
        let io_mapped = self.io.as_ref().is_some_and(QueuePair::has_mappings);
        if !self.admin.has_mappings() && !io_mapped {
            return true;
        }
        let mut installed = crate::iommu::installed();
        let Some(isolation) = installed.as_mut() else {
            self.admin.retain_mapped();
            if let Some(queue) = self.io.as_mut() {
                queue.retain_mapped();
            }
            return false;
        };
        let admin = self.admin.unmap_from(isolation);
        let io = self
            .io
            .as_mut()
            .is_none_or(|queue| queue.unmap_from(isolation));
        admin && io
    }

    /// Fence DMA, stop the engine, retire every queue and then retire the BAR.
    pub(crate) fn retire(
        mut self,
        device: &mut crate::pci::Device,
        context: &mut ThreadContext,
    ) -> ControllerRetirement {
        if let Err(reason) = device.disable_bus_master_for_retirement() {
            warn!("nvme: bus-master retirement fence refused: {reason}");
            self.stop_dma(None);
            return quarantine_controller(self);
        }
        self.stop_dma(None);
        if !self.dma.is_quiesced() {
            return quarantine_controller(self);
        }
        if !self.unmap_queues() {
            warn!("nvme: a queue mapping stayed reachable during retirement");
            return quarantine_controller(self);
        }
        let Some(registers) = self.registers.take() else {
            return quarantine_controller(self);
        };
        match mem::unmap_mapped_pci_bar_exact(context, registers) {
            mem::MappedPciBarRetirement::Retired => ControllerRetirement::Retired,
            mem::MappedPciBarRetirement::Retained(registers) => {
                self.registers = Some(registers);
                quarantine_controller(self)
            }
            mem::MappedPciBarRetirement::Quarantined => quarantine_controller(self),
        }
    }

    /// Force the ordinary timeout arm and prove its cleanup before returning.
    #[cfg(feature = "selftest")]
    pub(crate) fn a_timeout_quiesces_before_dma_returns(&mut self, data: &mut DmaSegment) -> bool {
        self.force_timeout = true;
        let result = self.read_blocks(0, 1, data);
        self.force_timeout = false;

        let protected = matches!(result, Err(BringUpError::NotReady))
            && self.dma.is_quiesced()
            && !data.is_retained();
        // Record the expected failure before quiescing the deliberately
        // outstanding DMA command for the remainder of the boot.
        if self.dma.is_in_flight() {
            self.stop_dma(Some(data));
        }
        protected
    }

    /// The queue pair with this identifier.
    fn queue_mut(&mut self, which: u16) -> Option<&mut QueuePair> {
        if which == self.admin.id {
            return Some(&mut self.admin);
        }
        self.io.as_mut().filter(|queue| queue.id == which)
    }

    /// Every frame the domain must reach while the controller idles.
    #[cfg(feature = "selftest")]
    pub(super) fn queue_frames(&self) -> Option<alloc::vec::Vec<u64>> {
        let mut frames = alloc::vec::Vec::new();
        self.admin.frames(&mut frames)?;
        if let Some(io) = self.io.as_ref() {
            io.frames(&mut frames)?;
        }
        Some(frames)
    }
}

/// Whether a submission put its transfer buffer into the domain.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DataMapping {
    Mapped,
    Unmapped,
}

/// Take a published buffer out of the domain, or report that it is still in.
fn unmap_data(isolation: &mut Vtd, data: Option<&mut DmaSegment>, mapping: DataMapping) -> bool {
    mapping == DataMapping::Unmapped
        || data.is_none_or(|segment| isolation.unmap_segment(segment).is_ok())
}
