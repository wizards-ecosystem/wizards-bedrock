use nvme::{
    Capabilities, Capacity, Command, Completion, NAMESPACE_IDENTIFY_BYTES, Namespace, PollLimit,
    ReadTransfer,
};
use wost::arch::current::vtd::Vtd;
use wost::mm::{self, DmaSegment};
use x86_paging::PAGE_SIZE;

use super::{
    BringUpError, COMMAND_TIMEOUT_MS, Controller, ControllerRetirement, IO_QUEUE, NAMESPACE,
    POLL_ATTEMPTS, QueuePair, clock_ns, quarantine_controller, stop_controller, write,
    write_submission,
};
use crate::thread::ThreadContext;
use crate::{mem, warn};

impl Controller {
    /// What `CAP` said.
    pub(crate) const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Entries in each queue.
    pub(crate) const fn queue_entries(&self) -> u32 {
        self.admin.capacity.entries()
    }

    /// Ask the namespace how large it is and how large its blocks are.
    pub(super) fn identify_namespace(&mut self, capacity: Capacity) -> Result<(), BringUpError> {
        let mut buffer =
            DmaSegment::from(mm::allocate_contiguous(1, 1).ok_or(BringUpError::NoMemory)?);
        if !buffer.zero() {
            return Err(BringUpError::NoMemory);
        }
        let address = buffer.address().ok_or(BringUpError::NoMemory)?.get();

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
        let negotiation =
            Command::number_of_queues(self.take_id(), 1, 1).ok_or(BringUpError::QueueSize)?;
        self.submit_admin(negotiation, None)?;

        let pair = QueuePair::new(IO_QUEUE, capacity).ok_or(BringUpError::NoMemory)?;
        // Owned before the first fallible domain transition: retirement must see an uncertain
        // partial map, and keeps device and domain quarantined until the exact queue leaves are
        // invalidated.
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
        let completion_address = pair
            .completion
            .address()
            .ok_or(BringUpError::NoMemory)?
            .get();
        let submission_address = pair
            .submission
            .address()
            .ok_or(BringUpError::NoMemory)?
            .get();
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
        if self.io.is_none() {
            return Err(BringUpError::NotReady);
        }
        let address = into.address().ok_or(BringUpError::NotReady)?.get();
        let command = Command::read(self.take_id(), NAMESPACE, address, PAGE_SIZE, request)
            .ok_or(BringUpError::NotReady)?;

        self.submit_on(IO_QUEUE, command, Some(into)).map(|_| ())
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
        self.fail_submitted(BringUpError::Timeout, data, isolation, mapping)
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
        warn!("nvme: {error}; the controller is stopped for the rest of the boot");
        self.stop_dma(data.as_deref_mut());
        if !self.dma.must_retain() && !unmap_data(isolation, data, mapping) {
            warn!("nvme: a quiesced transfer buffer stays mapped in the dma domain");
        }
        Err(error)
    }

    /// Fail closed after a completion made the transfer buffer safe.
    fn fail_controller<T>(&mut self, error: BringUpError) -> Result<T, BringUpError> {
        warn!("nvme: {error}; the controller is stopped for the rest of the boot");
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

    #[cfg(feature = "selftest")]
    pub(crate) fn a_timeout_quiesces_before_dma_returns(&mut self, data: &mut DmaSegment) -> bool {
        self.force_timeout = true;
        let result = self.read_blocks(0, 1, data);
        self.force_timeout = false;

        matches!(result, Err(BringUpError::Timeout))
            && self.dma.is_quiesced()
            && !data.is_retained()
    }

    /// The queue pair with this identifier.
    fn queue_mut(&mut self, which: u16) -> Option<&mut QueuePair> {
        if which == self.admin.id {
            return Some(&mut self.admin);
        }
        self.io.as_mut().filter(|queue| queue.id == which)
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
