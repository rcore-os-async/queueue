use core::mem::MaybeUninit;

// TODO: impl cancel

pub trait SlotLike : Default {
    type Item;

    fn push(&mut self, i: Self::Item) -> Result<(), Self::Item>;
    fn pop(&mut self) -> Option<Self::Item>;
    fn size(&self) -> usize;
}

// Asserts that N < 64
struct Level<S: SlotLike, const N: usize> {
    bitset: u64,
    slots: [S; N],
}

impl<S: SlotLike, const N: usize> Default for Level<S, N> {
    fn default() -> Self {
        unsafe {
            let mut slots: [S; N] = MaybeUninit::uninit().assume_init();

            for slot in slots.iter_mut() {
                core::ptr::write(slot, Default::default());
            }

            Self {
                bitset: 0,
                slots,
            }
        }
    }
}

impl<S: SlotLike, const N: usize> Level<S, N> {
    pub fn push_at(&mut self, at: u32, i: S::Item) -> Result<(), S::Item> {
        let ret = self.slots[at as usize].push(i);
        if ret.is_ok() {
            self.bitset |= 1 << at;
        }
        ret
    }

    pub fn pop_at(&mut self, at: u32) -> Option<S::Item> {
        let popped = self.slots[at as usize].pop();
        if self.slots[at as usize].size() == 0 {
            self.bitset &= !(1 << at);
        }
        popped
    }

    pub fn next_event(&self, from: u32) -> Option<u32> {
        if self.bitset == 0 {
            None
        } else {
            let zeros = self.bitset.trailing_zeros();
            assert!(zeros >= from);
            return Some(zeros);
        }
    }

    pub fn replace_slot(&mut self, idx: u32, slot: S) -> S {
        self.bitset &= !(1 << idx);
        core::mem::replace(&mut self.slots[idx as usize], slot)
    }

    pub fn drain_until<'a>(&'a mut self, bound: u32) -> LevelDrain<'a, S, N> {
        LevelDrain {
            level: self,
            until: bound
        }
    }

    pub fn drain<'a>(&'a mut self) -> LevelDrain<'a, S, N> {
        self.drain_until(N as u32)
    }
}

struct LevelDrain<'a, S: SlotLike, const N: usize> {
    level: &'a mut Level<S, N>,
    until: u32,
}

impl<'a, S: SlotLike, const N: usize> Iterator for LevelDrain<'a, S, N> {
    type Item = S::Item;
    fn next(&mut self) -> Option<Self::Item> {
        if self.level.bitset == 0 {
            return None;
        }

        let idx = self.level.bitset.trailing_zeros();

        if idx >= self.until {
            return None;
        }

        self.level.pop_at(idx)
    }
}

// CUTOFF should be less than 6 (or 64)
// LEVEL is recommended to be ceil(64 / CUTOFF)
pub struct Wheel<T, S: SlotLike<Item = (T, usize)>, const LEVEL: usize, const CUTOFF: usize> {
    elapsed: usize,
    levels: [WheelLevel<S, CUTOFF>; LEVEL],
}

impl<T, S: SlotLike<Item = (T, usize)>, const LEVEL: usize, const CUTOFF: usize> Wheel<T, S, LEVEL, CUTOFF> {
    pub fn new(at: usize) -> Self {
        unsafe {
            let mut levels: [WheelLevel<S, CUTOFF>; LEVEL] = MaybeUninit::uninit().assume_init();

            for level in levels.iter_mut() {
                core::ptr::write(level, Default::default());
            }

            Self {
                elapsed: at,
                levels,
            }
        }
    }

    pub fn schedule(&mut self, tick: usize, i: T) -> Result<(), T> {
        let (wheel, offset) = if let Some(inner) = self.get_pos(tick) {
            inner
        } else {
            return Err(i);
        };

        self.levels[wheel].push_at(offset, (i, tick)).map_err(|err| err.0)
    }

    fn get_pos(&mut self, tick: usize) -> Option<(usize, u32)> {
        assert!(tick < (1 << (CUTOFF * LEVEL)));

        if tick < self.elapsed {
            return None;
        }

        let same_leading = if self.elapsed == tick {
            63
        } else {
            (self.elapsed ^ tick).leading_zeros()
        };

        const BITCOUNT_TOT: usize = core::mem::size_of::<usize>() * 8;
        let first_one = BITCOUNT_TOT - same_leading as usize - 1;

        let wheel = first_one / CUTOFF;
        let offset = (tick >> (wheel * CUTOFF)) & ((1 << CUTOFF) - 1);
        Some((wheel, offset as u32))
    }

    pub fn elapsed(&self) -> usize {
        self.elapsed
    }

    pub fn fast_forward<F: FnMut(T, usize)>(&mut self, moment: usize, mut f: F) {
        assert!(moment > self.elapsed);

        if self.elapsed == moment {
            return;
        }

        let same_leading = (self.elapsed ^ moment).leading_zeros();
        const BITCOUNT_TOT: usize = core::mem::size_of::<usize>() * 8;
        let first_one = BITCOUNT_TOT - same_leading as usize - 1;
        let first_same_wheel = first_one / CUTOFF;

        #[cfg(test)]
        println!("CASCADE: {} => {}, level {}", self.elapsed, moment, first_same_wheel);

        // Cascade from first_one
        // Clear all bottom queues
        for i in 0..first_same_wheel {
            // Draining in place is faster than replacing
            for item in self.levels[i].drain() {
                f(item.0, item.1);
            }
        }

        // Clear skipped slots
        let idx_mask = (1 << CUTOFF) - 1;
        let from_idx = (self.elapsed >> (CUTOFF * first_same_wheel)) & idx_mask;
        let to_idx = (moment >> (CUTOFF * first_same_wheel)) & idx_mask;

        #[cfg(test)]
        println!("| {} -> {}", from_idx, to_idx);

        if to_idx != from_idx {
            // Implies to_idx > 0
            for item in self.levels[first_same_wheel].drain_until(to_idx as u32) { // Upperbound
                f(item.0, item.1)
            }
        }

        self.elapsed = moment;

        let mut cascading = self.levels[first_same_wheel].replace_slot(to_idx as u32, Default::default());
        while let Some((item, ts)) = cascading.pop() {
            if ts <= moment {
                f(item, ts);
            } else {
                self.schedule(ts, item);
            }
        }
    }

    // Get the minimal possible time for the next event
    pub fn min_next_event(&self) -> Option<usize> {
        let mut left = self.elapsed;

        for i in 0..LEVEL {
            let tail = (left & ((1<<CUTOFF) - 1)) as u32;
            left >>= CUTOFF;

            if let Some(ev) = self.levels[i].next_event(tail) {
                let ret_high = (left << CUTOFF) | ev as usize;
                return Some(ret_high << (i * CUTOFF));
            }
        }
        None
    }
}

pub struct BoundedSlot<T, const N: usize> {
    storage: [MaybeUninit<T>; {N}],
    size: usize,
}

impl<T, const N: usize> Default for BoundedSlot<T, N> {
    fn default() -> Self {
        Self {
            storage: MaybeUninit::uninit_array(),
            size: 0,
        }
    }
}

impl<T, const N: usize> SlotLike for BoundedSlot<T, {N}> {
    type Item = T;
    fn push(&mut self, i: Self::Item) -> Result<(), T> {
        if self.size == N {
            return Err(i);
        }

        unsafe {
            self.storage[self.size].as_mut_ptr().write(i);
        }
        self.size += 1;
        Ok(())
    }

    fn pop(&mut self) -> Option<Self::Item> { 
        if self.size == 0 {
            None
        } else {
            self.size -= 1;
            let result = unsafe { self.storage[self.size].as_ptr().read() };
            Some(result)
        }
    }

    fn size(&self) -> usize { 
        self.size
    }
}

#[cfg(any(feature="std", test))]
impl<T> SlotLike for std::collections::VecDeque<T> {
    type Item = T;

    fn push(&mut self, i: Self::Item) -> Result<(), Self::Item> {
        self.push_front(i);
        Ok(())
    }
    fn pop(&mut self) -> Option<Self::Item> {
        self.pop_back()
    }

    fn size(&self) -> usize {
        self.len()
    }
}

pub type BoundedWheel<T, const N: usize> = Wheel<T, BoundedSlot<(T, usize), N>, 8, 6>;

#[cfg(any(feature="std", test))]
pub type VecDequeWheel<T> = Wheel<T, std::collections::VecDeque<(T, usize)>, 8, 6>;

#[cfg(test)]
mod test {
    lazy_static::lazy_static! {
    }

    #[test]
    fn basic() {
        let wheel = Box::leak(box super::BoundedWheel::<usize, 16>::new(0));
        assert_eq!(wheel.min_next_event(), None);
        wheel.schedule(5, 1).unwrap();
        assert_eq!(wheel.min_next_event(), Some(5));
        wheel.schedule(6, 2).unwrap();
        wheel.schedule(89, 3).unwrap();
        wheel.schedule(100, 4).unwrap();
        wheel.schedule(90, 5).unwrap();
        wheel.schedule(129, 6).unwrap();
        wheel.schedule(170, 7).unwrap();
        assert_eq!(wheel.min_next_event(), Some(5));

        wheel.fast_forward(2, |_, _| panic!());
        wheel.fast_forward(4, |_, _| panic!());
        wheel.fast_forward(5, |item, at| {
            println!("{} @ {}", item, at);
            if item != 1 || at != 5 {
                panic!();
            }
        });
        assert_eq!(wheel.min_next_event(), Some(6));

        wheel.fast_forward(6, |item, at| {
            println!("{} @ {}", item, at);
            if item != 2 || at != 6 {
                panic!();
            }
        });
        assert_eq!(wheel.min_next_event(), Some(64));

        wheel.fast_forward(64, |_, _| panic!());
        assert_eq!(wheel.min_next_event(), Some(89));
        wheel.fast_forward(89, |item, at| {
            println!("{} @ {}", item, at);
            if item != 3 || at != 89 {
                panic!();
            }
        });

        assert_eq!(wheel.min_next_event(), Some(90));
        wheel.fast_forward(200, |item, at| {
            println!("{} @ {}", item, at);
        });
    }

    #[test]
    fn random() {
        use rand_distr::*;
        use rand_distr::Distribution;
        use rand::*;
        use std::collections::*;

        let distr = ChiSquared::new(65536f64).unwrap(); // Hey we just need a long tail distribution
        let rng = thread_rng();
        let mut generated = distr.sample_iter(rng)
            .take(65536)
            .enumerate()
            .map(|(idx, val)| (val.round() as usize + 1, idx)).collect::<Vec<(usize, usize)>>();
        
        generated.sort();
        // Now generated is sorted by moment
        println!("Pairs generated");
        
        let mut wheel = super::VecDequeWheel::new(0);
        
        let mut max = 0;
        println!("First 10 timings:");
        for (moment, idx) in generated.iter() {
            if *idx < 10 {
                println!("{} -> {}", idx, moment);
            }

            wheel.schedule(*moment, *idx).unwrap();
            if *moment > max {
                max = *moment;
            }
        }

        let mut rng = thread_rng();
        let pick = Binomial::new(1, 0.3).unwrap();
        let mut pending = HashMap::new();
        let mut last_val = 1;
        for (val, idx) in generated.into_iter() {
            assert!(last_val <= val);
            if last_val != val {
                // Changed
                if pick.sample(&mut rng) == 1 {
                    println!("Fast forward to {}", last_val);
                    wheel.fast_forward(last_val, |item, at| {
                        let removed = pending.remove(&item);
                        if removed != Some(at) {
                            panic!("Invalid timing! Expected {}, got {:?}", at, removed);
                        }
                    });

                    if pending.len() != 0 {
                        panic!("Schedule lost!");
                    }
                }

                last_val = val;
            }

            assert!(wheel.min_next_event() <= Some(val));
            pending.insert(idx, val);
        }

        wheel.fast_forward(max, |item, at| {
            if pending.remove(&item) != Some(at) {
                panic!("Invalid timing!");
            }
        });

        if pending.len() != 0 {
            panic!("Schedule lost!");
        }

        assert_eq!(wheel.min_next_event(), None);
    }
}

// Helper type
type WheelLevel<S: SlotLike, const CUTOFF: usize> = Level<S, {1 << CUTOFF}>;
