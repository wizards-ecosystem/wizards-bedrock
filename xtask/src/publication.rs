//! Source-bound weak-memory checks for the shared ring.

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Order {
        Relaxed,
        Acquire,
        Release,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Event {
        PayloadVisible,
        IndexVisible,
        IndexRead,
        PayloadRead,
    }

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root")
            .to_path_buf()
    }

    fn method<'a>(source: &'a str, name: &str) -> &'a str {
        let signatures = [
            format!("    pub fn {name}"),
            format!("    pub const fn {name}"),
            format!("fn {name}"),
        ];
        let start = signatures
            .iter()
            .filter_map(|signature| source.find(signature))
            .min()
            .unwrap_or_else(|| panic!("missing method `{name}`"));
        method_at(source, start)
    }

    fn method_at(source: &str, start: usize) -> &str {
        let rest = &source[start..];
        let end = ["\n    ///", "\n///"]
            .into_iter()
            .filter_map(|marker| rest.find(marker))
            .min()
            .unwrap_or(rest.len());
        &rest[..end]
    }

    fn publication_source() -> String {
        std::fs::read_to_string(root().join("wost/src/arch/x86_64/shared_memory.rs"))
            .expect("x86 shared-memory source")
    }

    fn order(shared: &str, primitive: &str, name: &str) -> Order {
        let wrapper = method(shared, name);
        let body = method(primitive, name);
        let (instruction, order) = match name {
            "load_acquire_u32" => ("mov {value:e}, dword ptr [{address}]", Order::Acquire),
            "store_release_u32" => ("mov dword ptr [{address}], {value:e}", Order::Release),
            _ => panic!("unsupported publication primitive `{name}`"),
        };
        assert!(
            wrapper.contains(&format!("shared_memory::{name}(address")),
            "`{name}` bypasses the x86 shared-memory primitive"
        );
        assert!(
            body.contains(instruction)
                && body.contains("options(nostack, preserves_flags)")
                && !body.contains("nomem")
                && !body.contains("readonly")
                && !body.contains("pure"),
            "`{name}` is not one indivisible x86 access behind a compiler memory barrier"
        );
        order
    }

    fn position(events: &[Event; 4], wanted: Event) -> usize {
        events
            .iter()
            .position(|event| *event == wanted)
            .expect("the permutation contains every event")
    }

    /// Enumerate every visibility/execution order that can yield a published
    /// index beside an old payload. Release constrains the two producer events;
    /// acquire constrains the two consumer events. The published observation
    /// itself requires index visibility before its read, and the stale payload
    /// requires its read before payload visibility.
    fn published_stale_is_allowed(store: Order, load: Order) -> bool {
        let events = [
            Event::PayloadVisible,
            Event::IndexVisible,
            Event::IndexRead,
            Event::PayloadRead,
        ];
        for a in 0..4 {
            for b in 0..4 {
                for c in 0..4 {
                    for d in 0..4 {
                        if a == b || a == c || a == d || b == c || b == d || c == d {
                            continue;
                        }
                        let order = [events[a], events[b], events[c], events[d]];
                        let payload_visible = position(&order, Event::PayloadVisible);
                        let index_visible = position(&order, Event::IndexVisible);
                        let index_read = position(&order, Event::IndexRead);
                        let payload_read = position(&order, Event::PayloadRead);
                        let release_ok = store != Order::Release || payload_visible < index_visible;
                        let acquire_ok = load != Order::Acquire || index_read < payload_read;
                        if release_ok
                            && acquire_ok
                            && index_visible < index_read
                            && payload_read < payload_visible
                        {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn sources() -> (String, String) {
        let root = root();
        (
            std::fs::read_to_string(root.join("wost/src/mm/mod.rs")).expect("wost mm source"),
            std::fs::read_to_string(root.join("kernel/src/queue.rs")).expect("queue source"),
        )
    }

    fn protocol_source() -> String {
        std::fs::read_to_string(root().join("libs/ring-proto/src/lib.rs"))
            .expect("ring protocol source")
    }

    fn between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
        let rest = source
            .split_once(start)
            .unwrap_or_else(|| panic!("missing source marker {start:?}"))
            .1;
        rest.split_once(end)
            .unwrap_or_else(|| panic!("missing source marker {end:?}"))
            .0
    }

    #[test]
    fn release_publication_excludes_an_old_payload() {
        let (shared, queue) = sources();
        let primitive = publication_source();
        let store = order(&shared, &primitive, "store_release_u32");
        let load = order(&shared, &primitive, "load_acquire_u32");

        assert!(
            method(&queue, "publish_index").contains("store_release_u32"),
            "the queue bypasses the release-store boundary"
        );
        assert!(
            published_stale_is_allowed(Order::Relaxed, load),
            "the model cannot exhibit the release-side defect"
        );
        assert!(
            !published_stale_is_allowed(store, load),
            "the published index permits an old payload"
        );
    }

    #[test]
    fn acquire_consumption_excludes_an_early_payload_read() {
        let (shared, queue) = sources();
        let primitive = publication_source();
        let store = order(&shared, &primitive, "store_release_u32");
        let load = order(&shared, &primitive, "load_acquire_u32");

        assert!(
            method(&queue, "load_index").contains("load_acquire_u32"),
            "the queue bypasses the acquire-load boundary"
        );
        assert!(
            published_stale_is_allowed(store, Order::Relaxed),
            "the model cannot exhibit the acquire-side defect"
        );
        assert!(
            !published_stale_is_allowed(store, load),
            "the payload read may precede the acquired index"
        );
    }

    #[test]
    fn every_shared_payload_copy_is_volatile() {
        let (shared, queue) = sources();
        let byte_write = between(
            &shared,
            "fn write_byte_volatile(&self, offset: usize, byte: u8) -> bool",
            "fn read_byte_volatile(&self, offset: usize) -> Option<u8>",
        );
        let byte_read = between(
            &shared,
            "fn read_byte_volatile(&self, offset: usize) -> Option<u8>",
            "fn fill_volatile(&self, byte: u8) -> bool",
        );
        let span_write = between(
            &shared,
            "fn write_volatile_at(&self, offset: usize, bytes: &[u8])",
            "fn read_volatile_at(&self, offset: usize, into: &mut [u8])",
        );
        let span_read = between(
            &shared,
            "fn read_volatile_at(&self, offset: usize, into: &mut [u8])",
            "#[cfg(feature = \"selftest\")]",
        );
        let write = between(
            &shared,
            "fn write_shared_volatile",
            "fn read_shared_volatile",
        );
        let read = between(
            &shared,
            "fn read_shared_volatile",
            "impl From<Segment> for USegment",
        );
        assert!(
            byte_write.contains("if offset >= self.len {")
                && byte_write.contains("return false;")
                && byte_write.contains("self.base.wrapping_add(offset)")
                && byte_write.contains("destination.write_volatile(byte)"),
            "shared payload byte writes are not bounds-checked volatile operations"
        );
        assert!(
            span_write.contains("AccessRange::repeated(offset, bytes.len(), 1, self.len)")
                && span_write.contains("self.write_byte_volatile(offset, byte)")
                && write.contains("DirectSpan::from_shared(allocation)")
                && write.contains("span.write_volatile_at(offset, bytes)"),
            "shared payload writes bypass the checked volatile byte primitive"
        );
        assert!(
            byte_read.contains("if offset >= self.len {")
                && byte_read.contains("return None;")
                && byte_read.contains("self.base.wrapping_add(offset)")
                && byte_read.contains("source.read_volatile()"),
            "shared payload byte reads are not bounds-checked volatile operations"
        );
        assert!(
            span_read.contains("AccessRange::repeated(offset, into.len(), 1, self.len)")
                && span_read.contains("self.read_byte_volatile(offset)")
                && read.contains("DirectSpan::from_shared(allocation)")
                && read.contains("span.read_volatile_at(offset, into)"),
            "shared payload reads bypass the checked volatile byte primitive"
        );
        let raw = between(&shared, "impl Shared {", "impl SharedBytes {");
        assert!(
            raw.contains("pub fn into_bytes") && raw.contains("pub fn into_ring"),
            "raw Shared does not require one consuming partition choice"
        );
        assert!(
            !raw.contains("write_volatile_at")
                && !raw.contains("read_volatile_at")
                && !raw.contains("load_acquire_u32")
                && !raw.contains("store_release_u32"),
            "raw Shared retains an access door"
        );
        assert!(
            method(&shared, "write_payload_at").contains(
                "if !self.layout.contains_payload(offset, bytes.len()) {\n            return false;\n        }"
            ) && method(&shared, "read_payload_at").contains(
                "if !self.layout.contains_payload(offset, into.len()) {\n            return false;\n        }"
            ),
            "a ring payload copy bypasses its checked byte partition"
        );
        assert!(
            method(&shared, "store_release_u32").contains(
                "if !self.layout.contains_publication_word(offset) {\n            return false;\n        }"
            ) && method(&shared, "load_acquire_u32").contains(
                "if !self.layout.contains_publication_word(offset) {\n            return None;\n        }"
            ),
            "a ring publication access bypasses its checked partition"
        );
        assert!(
            queue.contains(".write_payload_at(") && queue.contains(".read_payload_at("),
            "the queue does not use the volatile payload boundary"
        );
        assert!(
            !queue.contains(".write_at(") && !queue.contains(".read_at("),
            "the queue still performs an ordinary shared-page copy"
        );
    }

    #[test]
    fn publication_words_cross_the_hostile_alias_boundary_without_a_rust_object() {
        let (shared, _) = sources();
        let primitive = publication_source();
        let boundary = between(
            &shared,
            "    pub fn load_acquire_u32",
            "    /// Retain the allocation without exposing its bytes.",
        );
        let load = method(&shared, "load_acquire_u32");
        let store = method(&shared, "store_release_u32");
        let address = method(&shared, "publication_address");

        assert!(
            !boundary.contains("AtomicU32")
                && !boundary.contains("&*")
                && !boundary.contains("*const")
                && !boundary.contains("*mut")
                && !boundary.contains(".read")
                && !boundary.contains(".write")
                && !boundary.contains("ptr::read")
                && !boundary.contains("ptr::write")
                && !boundary.contains("transmute")
                && !boundary.contains("read_volatile")
                && !boundary.contains("write_volatile"),
            "a publication word became a Rust atomic/reference or a non-atomic volatile access"
        );
        assert!(
            boundary.contains("shared_memory::load_acquire_u32(address)")
                && boundary.contains("shared_memory::store_release_u32(address, value.to_le())"),
            "a publication access bypasses the external-memory primitive"
        );
        assert!(
            load.contains(
                "if !self.layout.contains_publication_word(offset) {\n            return None;\n        }"
            ) && store.contains(
                "if !self.layout.contains_publication_word(offset) {\n            return false;\n        }"
            ),
            "a publication access bypasses its immutable layout partition"
        );
        assert!(
            address.contains("let end = offset.checked_add(size_of::<u32>())?;")
                && address.contains(
                    "if end as u64 > self.len() {\n            return None;\n        }"
                )
                && address.contains("direct_pointer(self.address(), self.len())")
                && address.contains("base.addr().checked_add(offset)")
                && address.contains(
                    "if !address.is_multiple_of(align_of::<u32>()) {\n            return None;\n        }"
                ),
            "the external-memory primitive lacks a complete bounds/alignment preflight"
        );
        assert!(
            !primitive.contains("AtomicU32")
                && !primitive.contains("read_volatile")
                && !primitive.contains("write_volatile")
                && !primitive.contains("&*")
                && !primitive.contains("*const")
                && !primitive.contains("*mut")
                && !primitive.contains(".read")
                && !primitive.contains(".write")
                && !primitive.contains("ptr::read")
                && !primitive.contains("ptr::write")
                && !primitive.contains("transmute"),
            "the x86 publication primitive re-entered Rust's object or volatile model"
        );
        assert_eq!(
            order(&shared, &primitive, "load_acquire_u32"),
            Order::Acquire
        );
        assert_eq!(
            order(&shared, &primitive, "store_release_u32"),
            Order::Release
        );
    }

    #[test]
    fn kernel_completion_producers_publish_in_tail_order() {
        let (_, queue) = sources();
        let post = method(&queue, "post");
        let complete = post.find("ring.protocol.complete()").expect("CQ advance");
        let payload = post
            .find("ring.pages.write_payload_at")
            .expect("CQ payload");
        let publish = post
            .find("publish_index(&ring.pages, header::COMPLETION_TAIL, tail)")
            .expect("CQ tail publication");

        assert_eq!(
            post.matches("self.ring.lock_leaf()").count(),
            1,
            "completion state and page publication use separate critical sections"
        );
        assert!(
            complete < payload && payload < publish,
            "a completion tail can pass the entry it publishes"
        );
    }

    #[test]
    fn admission_reserves_before_the_shared_submission_is_consumed() {
        let (_, queue) = sources();
        let protocol = protocol_source();
        let submit = method(&protocol, "submit");
        let produce = submit
            .find("self.submission.produce()")
            .expect("submission admission");
        let reserve = submit
            .find("self.reserved += 1;")
            .expect("completion reservation");
        assert!(
            produce < reserve,
            "the completion reservation can precede refused admission"
        );

        let take_start = queue
            .find("    fn take(&self, snapshot: &Snapshot)")
            .expect("queue submission take");
        let take = method_at(&queue, take_start);
        let reserve = take
            .find("ring.protocol.submit()?")
            .expect("reservation at admission");
        let consume = take
            .find("ring.protocol.take()")
            .expect("submission consumption");
        let payload = take
            .find("ring.pages.read_payload_at")
            .expect("submission payload read");
        let publish = take
            .find("publish_index(&ring.pages, header::SUBMISSION_HEAD, head)")
            .expect("submission-head publication");
        assert!(
            reserve < consume && consume < payload && payload < publish,
            "admission, payload consumption and slot release do not follow ownership order"
        );
    }

    #[test]
    fn every_terminal_path_claims_one_settlement_until_publication() {
        let (_, queue) = sources();
        let begin = method(&queue, "begin_settlement");
        assert_eq!(
            begin.matches("self.pending.lock_leaf()").count(),
            1,
            "settlement eligibility and its claim use different locks"
        );
        let guard = begin
            .find("if !entry.settling && wanted(*entry)")
            .expect("one-shot settlement guard");
        let claim = begin
            .find("entry.settling = true;")
            .expect("settlement claim");
        assert!(
            guard < claim,
            "a terminal path publishes before owning the entry"
        );

        let settle = method(&queue, "settle_pending");
        let publish = settle.find("self.post(Completion").expect("terminal post");
        let release = settle
            .find("pending.slots[index] = None;")
            .expect("pending release");
        let restore = settle
            .find("settling: false")
            .expect("refused-post restoration");
        assert!(
            publish < release && publish < restore,
            "pending ownership can end before terminal publication answers"
        );

        for path in ["cancel", "expire", "abandon"] {
            let body = method(&queue, path);
            let begin = body
                .find("self.begin_settlement")
                .unwrap_or_else(|| panic!("`{path}` bypasses settlement ownership"));
            let settle = body
                .find("self.settle_pending")
                .unwrap_or_else(|| panic!("`{path}` bypasses terminal publication"));
            assert!(
                begin < settle,
                "`{path}` publishes before claiming ownership"
            );
        }
    }
}
