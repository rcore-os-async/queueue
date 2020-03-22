use super::sequencer::Sequencer;

use core::cell::UnsafeCell;
use core::sync::atomic::*;
use core::mem::MaybeUninit;

pub struct Slot<T, S: Sequencer> {
    data: UnsafeCell<MaybeUninit<T>>,
    occupied: AtomicBool,
    seq: S,
}

impl<T, S: Sequencer> Slot<T, S> {
    pub fn push(&self, data: T, seq: usize) {
        // Wait until sequence number
        // timeout = None asserts Ok(())
        // seq * 2 = push
        self.seq.wait_until(seq * 2, None).unwrap();

        // Wait until occupied = false
        // It's highly improbable that we have so much thread preempeted
        //   between pop_ticket -> pop and push_ticket -> push, that the ticket wraps around
        // So we are doing a simple spin here.

        // AcqRel, because we don't want it to be reordered before we got the sequence number,
        //   and we don't want it to be reordered after we actually stores the data
        // TODO: maybe we can make this one less strict? because seq.wait_until already has acquire schematic
        while self.occupied.compare_and_swap(false, true, Ordering::AcqRel) { }

        // Now self.data is invalid memory. So we can write into it without dropping the data inside
        unsafe{ core::ptr::write(self.data.get(), MaybeUninit::new(data)) };

        // Bump sequence number
        self.seq.update_next(seq * 2 + 1);
    }

    pub fn pop(&self, seq: usize) -> T {
        self.seq.wait_until(seq * 2 + 1, None).unwrap();
        let result = unsafe { core::ptr::read(self.data.get()).assume_init() };
        self.occupied.store(false, Ordering::Release);
        self.seq.update_next(seq * 2 + 2);

        result
    }
}

unsafe impl<T, S: Sequencer> Send for Slot<T, S> {}
unsafe impl<T, S: Sequencer> Sync for Slot<T, S> {}

impl<T, S: Sequencer> Default for Slot<T, S> {
    fn default() -> Self {
        Self {
            data: UnsafeCell::new(MaybeUninit::uninit()),
            occupied: AtomicBool::new(false),
            seq: S::default(),
        }
    }
}
