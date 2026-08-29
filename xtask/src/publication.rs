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
        let end = rest.find("\n    ///").unwrap_or(rest.len());
        &rest[..end]
    }

    fn order(source: &str, name: &str) -> Order {
        let body = method(source, name);
        let found: Vec<Order> = [
            ("Ordering::Relaxed", Order::Relaxed),
            ("Ordering::Acquire", Order::Acquire),
            ("Ordering::Release", Order::Release),
        ]
        .into_iter()
        .filter_map(|(spelling, order)| body.contains(spelling).then_some(order))
        .collect();
        assert_eq!(
            found.len(),
            1,
            "`{name}` must contain exactly one supported atomic order"
        );
        found[0]
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

    fn shared_impl(source: &str) -> &str {
        let start = source.find("impl Shared {").expect("Shared implementation");
        let rest = &source[start..];
        let end = rest
            .find("impl From<Segment> for USegment")
            .expect("end of Shared implementation");
        &rest[..end]
    }

    #[test]
    fn release_publication_excludes_an_old_payload() {
        let (shared, queue) = sources();
        let store = order(&shared, "store_release_u32");
        let load = order(&shared, "load_acquire_u32");

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
        let store = order(&shared, "store_release_u32");
        let load = order(&shared, "load_acquire_u32");

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
        assert!(
            method(&shared, "write_volatile_at").contains("write_volatile(byte)"),
            "shared payload writes are ordinary memory operations"
        );
        assert!(
            method(&shared, "read_volatile_at").contains("read_volatile()"),
            "shared payload reads are ordinary memory operations"
        );
        let boundary = shared_impl(&shared);
        assert!(
            !boundary.contains("pub fn write_at") && !boundary.contains("pub fn read_at"),
            "Shared still offers an ordinary-copy bypass"
        );
        assert!(
            queue.contains(".write_volatile_at(") && queue.contains(".read_volatile_at("),
            "the queue does not use the volatile payload boundary"
        );
        assert!(
            !queue.contains(".write_at(") && !queue.contains(".read_at("),
            "the queue still performs an ordinary shared-page copy"
        );
    }

    #[test]
    fn kernel_completion_producers_publish_in_tail_order() {
        let (_, queue) = sources();
        let post = method(&queue, "post");
        let complete = post.find("ring.protocol.complete()").expect("CQ advance");
        let payload = post
            .find("ring.pages.write_volatile_at")
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
            .find("ring.pages.read_volatile_at")
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
