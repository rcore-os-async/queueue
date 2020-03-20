use crate::slot::Slot;
use crate::sequencer::Sequencer;

use core::sync::atomic::*;
use core::result::Result;
use core::mem::MaybeUninit;

pub trait Queue: Send + Sync {
    type Item;

    unsafe fn sync_push(&self, t: Self::Item) -> Result<(), Self::Item> {
        self.shared_push(t)
    }
    unsafe fn sync_pop(&self) -> Option<Self::Item> {
        self.shared_pop()
    }

    fn shared_push(&self, t: Self::Item) -> Result<(), Self::Item>;
    fn shared_pop(&self) -> Option<Self::Item>;

    fn producer<'a>(&'a self) -> Producer<'a, Self> where Self: Sized{
        Producer {
            queue: self,
        }
    }

    fn consumer<'a>(&'a self) -> Consumer<'a, Self> where Self: Sized {
        Consumer {
            queue: self,
        }
    }
}

pub struct StaticQueue<T, S: Sequencer, const N: usize> {
    slots: [Slot<T, S>; {N}],

    push_ticket: AtomicUsize,
    pop_ticket: AtomicUsize,
}

impl<T, S: Sequencer, const N: usize> StaticQueue<T, S, {N}> {
    fn obtain_push_ticket(&self) -> Option<usize> {
        loop {
            let cur_push = self.push_ticket.load(Ordering::Acquire);
            let cur_pop = self.pop_ticket.load(Ordering::Acquire);

            let size = cur_push as isize - cur_pop as isize;
            // Queue is full
            if size >= {N} as isize {
                break None;
            }

            if self.push_ticket.compare_and_swap(cur_push, cur_push + 1, Ordering::AcqRel) == cur_push {
                break Some(cur_push);
            }
        }
    }

    fn obtain_pop_ticket(&self) -> Option<usize> {
        loop {
            let cur_pop = self.pop_ticket.load(Ordering::Acquire);
            let cur_push = self.push_ticket.load(Ordering::Acquire);

            if cur_pop >= cur_push {
                // It's possible that cur_pop > cur_push because hey, memory ordering.
                // Maybe a race between three threads?

                return None;
            }

            if self.pop_ticket.compare_and_swap(cur_pop, cur_pop + 1, Ordering::AcqRel) == cur_pop {
                break Some(cur_pop);
            }
        }
    }
}

impl<T, S: Sequencer, const N: usize> Queue for StaticQueue<T, S, {N}> {
    type Item = T;

    fn shared_pop(&self) -> Option<Self::Item> {
        let ticket = self.obtain_pop_ticket()?;

        let offset = ticket % N;
        let seq = ticket / N;

        Some(self.slots[offset].pop(seq))
    }

    fn shared_push(&self, t: Self::Item) -> Result<(), Self::Item> {
        let ticket = match self.obtain_push_ticket() {
            None => return Err(t),
            Some(ticket) => ticket,
        };

        let offset = ticket % N;
        let seq = ticket / N;

        self.slots[offset].push(t, seq);

        Ok(())
    }
}

impl<T, S: Sequencer, const N: usize> Default for StaticQueue<T, S, {N}> {
    fn default() -> Self {
        unsafe { MaybeUninit::zeroed().assume_init() }
    }
}

#[derive(Clone)]
pub struct Consumer<'a, Q: Queue> {
    queue: &'a Q,
}

#[derive(Clone)]
pub struct Producer<'a, Q: Queue> {
    queue: &'a Q,
}

impl<'a, Q: Queue> Consumer<'a, Q> {
    pub fn pop(&mut self) -> Option<Q::Item> {
        self.queue.shared_pop()
    }
}

impl<'a, Q: Queue> Producer<'a, Q> {
    pub fn push(&mut self, data: Q::Item) -> Result<(), Q::Item> {
        self.queue.shared_push(data)
    }
}

pub type StaticSpinQueue<T, const N: usize> = StaticQueue<T, crate::sequencer::SpinSequencer, {N}>;

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn basic() {
        let queue: StaticSpinQueue<usize, 4> = Default::default();

        let mut producer = queue.producer();
        let mut consumer = queue.consumer();

        producer.push(1).unwrap();
        producer.push(2).unwrap();
        producer.push(3).unwrap();
        producer.push(4).unwrap();
        let data = producer.push(5).unwrap_err();
        assert_eq!(data, 5);

        assert_eq!(consumer.pop(), Some(1));
        assert_eq!(consumer.pop(), Some(2));
        assert_eq!(consumer.pop(), Some(3));

        producer.push(5).unwrap();
        producer.push(6).unwrap();
        producer.push(7).unwrap();
        let data = producer.push(8).unwrap_err();
        assert_eq!(data, 8);

        assert_eq!(consumer.pop(), Some(4));
        assert_eq!(consumer.pop(), Some(5));
        assert_eq!(consumer.pop(), Some(6));
        assert_eq!(consumer.pop(), Some(7));
        assert_eq!(consumer.pop(), None);
        assert_eq!(consumer.pop(), None);
        assert_eq!(consumer.pop(), None);

        producer.push(8).unwrap();
        assert_eq!(consumer.pop(), Some(8));
    }

    #[test]
    fn spsc() {
        const RANGE: core::ops::Range<usize> = 0usize..65536usize;

        let queue: Box<StaticSpinQueue<usize, 4>> = Default::default();
        let queue = Box::leak(queue);

        let mut producer = queue.producer();
        let mut consumer = queue.consumer();

        let pth = std::thread::spawn(move || {
            for i in RANGE {
                loop {
                    if producer.push(i).is_ok() {
                        break;
                    }
                }
            }
        });

        let cth = std::thread::spawn(move || {
            for i in RANGE {
                loop {
                    match consumer.pop() {
                        None => continue,
                        Some(j) if j == i => break,
                        Some(j) => panic!("Unexpected item {}. Was waiting for {}.", j, i),
                    }
                }
            }
        });

        pth.join().unwrap();
        cth.join().unwrap();
    }

    #[test]
    fn mpmc() {
        use std::collections::HashMap;

        const RANGE: core::ops::Range<usize> = 0usize..65536usize;
        const P_COUNT: usize = 4;
        const C_COUNT: usize = 4;
        const C_CHECK_INTERVAL: usize = 128;

        let queue: Box<StaticSpinQueue<usize, 4>> = Default::default();
        let queue = Box::leak(queue);

        let pending_producer = Box::leak(Box::new(AtomicUsize::new(P_COUNT)));

        let mut pths = Vec::with_capacity(P_COUNT);
        for _ in 0..P_COUNT {
            let mut producer = queue.producer();
            let ppcnt = &*pending_producer;
            pths.push(std::thread::spawn(move || {
                for i in RANGE {
                    loop {
                        if producer.push(i).is_ok() {
                            break;
                        }
                    }
                }

                ppcnt.fetch_sub(1, Ordering::Release);
            }));
        }

        let mut cths = Vec::with_capacity(C_COUNT);

        for _ in 0..C_COUNT {
            let mut consumer = queue.consumer();
            let ppcnt = &*pending_producer;
            cths.push(std::thread::spawn(move || {
                let mut counter = HashMap::new();

                loop {
                    let pending = ppcnt.load(Ordering::Acquire);
                    if pending == 0 {
                        return counter;
                    }

                    for _ in 0..C_CHECK_INTERVAL {
                        if let Some(i) = consumer.pop() {
                            (*counter.entry(i).or_insert(0)) += 1;
                        }
                    }
                }
            }))
        }

        let mut tot = HashMap::new();

        for p in pths.into_iter() {
            p.join().unwrap();
        }

        for c in cths.into_iter() {
            let cnt = c.join().unwrap();
            for (k, v) in cnt.into_iter() {
                *tot.entry(k).or_insert(0) += v;
            }
        }

        let mut stdans = HashMap::new();
        for i in RANGE {
            stdans.insert(i, P_COUNT);
        }

        assert_eq!(stdans, tot);
    }
}