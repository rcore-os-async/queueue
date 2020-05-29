use core::sync::atomic::*;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

use queueue::queue::nonblocking::DynamicSpinQueue;
use queueue::queue::nonblocking::Queue;

fn sp_enqueue_bench(b: &mut Criterion) {
    let mut queue: DynamicSpinQueue<usize, 16> = Default::default();
    let mut producer = queue.producer();
    b.bench_function("Enqueue 1000", |b| {
        b.iter(|| {
            for i in 0..1000 {
                let zero = black_box(0);
                black_box(producer.push(zero));
            }
        })
    });
}

fn sc_dequeue_bench(b: &mut Criterion) {
    let mut queue: DynamicSpinQueue<usize, 16> = Default::default();
    let mut consumer = queue.consumer();
    b.bench_function("Dequeue 1000", |b| {
        b.iter(|| {
            for i in 0..1000 {
                consumer.pop();
            }
        })
    });
}

static stop_sig: AtomicBool = AtomicBool::new(false);

fn mp_enqueue_bench(b: &mut Criterion) {
    let queue: &'static mut DynamicSpinQueue<usize, 16> = Box::leak(Box::new(Default::default()));
    const P_COUNT: usize = 4;

    stop_sig.store(false, Ordering::Release);

    let mut handles = Vec::new();
    for _ in 0..P_COUNT - 1 {
        let mut producer = queue.producer();
        let handle = std::thread::spawn(move || {
            while !stop_sig.load(Ordering::Acquire) {
                for i in 0..1000 {
                    black_box(producer.push(black_box(0)));
                }
            }
        });
        handles.push(handle);
    }

    let mut producer = queue.producer();
    b.bench_function("MP Enqueue 1000", |b| {
        b.iter(|| {
            for i in 0..1000 {
                black_box(producer.push(black_box(0)));
            }
        })
    });

    stop_sig.store(true, Ordering::Release);
    for handle in handles.into_iter() {
        handle.join().unwrap();
    }
}

fn balenced_bench(b: &mut Criterion) {
    let queue: &'static mut DynamicSpinQueue<usize, 16> = Box::leak(Box::new(Default::default()));
    const PC_COUNT: usize = 4;

    stop_sig.store(false, Ordering::Release);

    let mut handles = Vec::new();
    for _ in 0..PC_COUNT - 1 {
        let mut producer = queue.producer();
        let handle = std::thread::spawn(move || {
            while !stop_sig.load(Ordering::Acquire) {
                for i in 0..1000 {
                    black_box(producer.push(black_box(0)));
                }
            }
        });
        handles.push(handle);
    }

    for _ in 0..PC_COUNT {
        let mut consumer = queue.consumer();
        let handle = std::thread::spawn(move || {
            while !stop_sig.load(Ordering::Acquire) {
                for i in 0..1000 {
                    black_box(consumer.pop());
                }
            }
        });
        handles.push(handle);
    }

    let mut producer = queue.producer();
    b.bench_function("Balanced", |b| {
        b.iter(|| {
            for i in 0..1000 {
                black_box(producer.push(black_box(0)));
            }
        })
    });

    stop_sig.store(true, Ordering::Release);
    for handle in handles.into_iter() {
        handle.join().unwrap();
    }
}

criterion_group!(
    benches,
    sp_enqueue_bench,
    sc_dequeue_bench,
    mp_enqueue_bench,
    balenced_bench
);
criterion_main!(benches);
