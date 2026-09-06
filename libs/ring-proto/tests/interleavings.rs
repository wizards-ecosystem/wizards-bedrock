//! Scheduled producer, worker, and consumer calls through the real ring model.
//!
//! Shuttle explores call ordering under sequential consistency; the TLA+
//! model owns weak-memory ordering (D-112).

use ring_proto::{Capacity, Queues};
use shuttle::sync::{Arc, Mutex};
use shuttle::thread;

fn capacity(entries: u32) -> Capacity {
    Capacity::new(entries).expect("a power-of-two capacity")
}

#[derive(Debug)]
struct State {
    queues: Queues,
    admitted: usize,
    taken: usize,
    completed: usize,
    reaped: usize,
}

fn exercise_production_queues() {
    let queues = Arc::new(Mutex::new(State {
        queues: Queues::new(capacity(2), capacity(4)).expect("a valid pair"),
        admitted: 0,
        taken: 0,
        completed: 0,
        reaped: 0,
    }));

    let producer_queues = Arc::clone(&queues);
    let producer = thread::spawn(move || {
        for _ in 0..4 {
            let mut state = producer_queues.lock().expect("producer lock");
            if state.queues.submit().is_some() {
                state.admitted += 1;
            }
            assert!(state.queues.is_consistent());
            drop(state);
            thread::yield_now();
        }
    });

    let worker_queues = Arc::clone(&queues);
    let worker = thread::spawn(move || {
        for _ in 0..8 {
            let took = {
                let mut state = worker_queues.lock().expect("worker take lock");
                let took = state.queues.take().is_some();
                if took {
                    state.taken += 1;
                }
                assert!(state.queues.is_consistent());
                took
            };
            thread::yield_now();
            if took {
                let mut state = worker_queues.lock().expect("worker completion lock");
                assert!(
                    state.queues.complete().is_some(),
                    "an admitted operation was dropped"
                );
                state.completed += 1;
                assert!(state.queues.is_consistent());
            }
            thread::yield_now();
        }
    });

    let consumer_queues = Arc::clone(&queues);
    let consumer = thread::spawn(move || {
        for _ in 0..8 {
            let mut state = consumer_queues.lock().expect("consumer lock");
            if state.queues.reap().is_some() {
                state.reaped += 1;
            }
            assert!(state.queues.is_consistent());
            drop(state);
            thread::yield_now();
        }
    });

    producer.join().expect("producer finished");
    worker.join().expect("worker finished");
    consumer.join().expect("consumer finished");

    let mut state = queues.lock().expect("final lock");
    while state.queues.take().is_some() {
        state.taken += 1;
        assert!(
            state.queues.complete().is_some(),
            "an admitted operation was dropped"
        );
        state.completed += 1;
    }
    while state.queues.reap().is_some() {
        state.reaped += 1;
    }
    assert!(state.admitted > 0, "no production submission was admitted");
    assert_eq!(state.taken, state.admitted, "an admission was not taken");
    assert_eq!(
        state.completed, state.admitted,
        "an admission was not completed"
    );
    assert_eq!(state.reaped, state.admitted, "a completion was not reaped");
    assert_eq!(state.queues.outstanding(), 0);
    assert!(state.queues.submissions().is_empty());
    assert!(state.queues.completions().is_empty());
    assert!(state.queues.is_consistent());
}

#[test]
fn scheduled_calls_preserve_the_production_reservation() {
    shuttle::check_random(exercise_production_queues, 10_000);
}
