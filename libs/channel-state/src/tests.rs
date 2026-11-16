use super::*;

const RACING: [CallEvent; 7] = [
    CallEvent::Deliver,
    CallEvent::Claim,
    CallEvent::Complete,
    CallEvent::Abandon,
    CallEvent::Cancel,
    CallEvent::Timeout,
    CallEvent::Close,
];

fn position(order: &[CallEvent], event: CallEvent) -> usize {
    order
        .iter()
        .position(|&candidate| candidate == event)
        .expect("every racing event is in the order")
}

// A reply right exists only once the call is delivered, and a reply completes only once claimed.
fn is_causal(order: &[CallEvent]) -> bool {
    let deliver = position(order, CallEvent::Deliver);
    deliver < position(order, CallEvent::Claim)
        && deliver < position(order, CallEvent::Abandon)
        && position(order, CallEvent::Claim) < position(order, CallEvent::Complete)
}

fn orders(events: &mut [CallEvent], taken: usize, out: &mut Vec<Vec<CallEvent>>) {
    if taken == events.len() {
        out.push(events.to_vec());
        return;
    }
    for pick in taken..events.len() {
        events.swap(taken, pick);
        orders(events, taken + 1, out);
        events.swap(taken, pick);
    }
}

#[test]
fn a_call_reaches_exactly_one_outcome_under_every_order_of_its_racing_events() {
    let mut all = Vec::new();
    let mut racing = RACING;
    orders(&mut racing, 0, &mut all);
    assert_eq!(all.len(), 5040, "every order of seven events");

    for order in all.iter().filter(|order| is_causal(order)) {
        let committed = Cell::IDLE
            .next(CallEvent::Commit, 0)
            .expect("an idle cell takes a call");
        let mut cell = committed;
        let mut outcomes = 0;
        let mut delivered = false;
        for &event in order {
            assert_eq!(
                cell.next(event, cell.generation() + 1),
                Err(CellRefusal::Stale),
                "{order:?}: {event:?} for a later call"
            );
            let Ok(next) = cell.next(event, cell.generation()) else {
                continue;
            };
            assert!(
                !cell.state().is_outcome(),
                "{order:?}: {event:?} changed an outcome"
            );
            if event == CallEvent::Deliver {
                delivered = true;
            }
            if next.state().is_outcome() {
                outcomes += 1;
            }
            cell = next;
        }
        assert_eq!(outcomes, 1, "{order:?} ended in {:?}", cell.state());
        assert!(
            !(delivered && cell.state() == CallState::PeerClosed),
            "{order:?}: a delivered call was reported peer closed"
        );

        let idle = cell
            .next(CallEvent::Consume, cell.generation())
            .expect("the caller consumes the outcome");
        assert_eq!(idle.state(), CallState::Idle);
        for &event in &RACING {
            assert_eq!(
                idle.next(event, committed.generation()),
                Err(CellRefusal::Stale),
                "{order:?}: {event:?} reached the next call"
            );
        }
    }
}
