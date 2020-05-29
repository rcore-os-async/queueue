use core::sync::atomic::*;

pub trait Sequencer: Default {
    fn wait_until(&self, sequence: usize, timeout: Option<core::time::Duration>) -> Result<(), ()>;
    fn update_next(&self, sequence: usize);
}

#[derive(Default)]
pub struct SpinSequencer {
    seq: AtomicUsize,
}

impl Sequencer for SpinSequencer {
    fn wait_until(&self, sequence: usize, timeout: Option<core::time::Duration>) -> Result<(), ()> {
        if timeout.is_some() {
            unimplemented!("Sorry, no timeout plz");
        }

        loop {
            if self.seq.load(Ordering::Acquire) == sequence {
                break Ok(());
            }
        }
    }

    fn update_next(&self, sequence: usize) {
        self.seq.store(sequence, Ordering::Release);
    }
}

#[cfg(feature = "std")]
#[derive(Debug, Default)]
pub struct CondvarSequencer {
    seq: std::sync::Mutex<usize>,
    condvar: std::sync::Condvar,
}

#[cfg(feature = "std")]
impl Sequencer for CondvarSequencer {
    fn wait_until(&self, sequence: usize, timeout: Option<core::time::Duration>) -> Result<(), ()> {
        let cur = self.seq.lock().unwrap();

        if *cur == sequence {
            return Ok(());
        }

        let cond = |pending: &mut usize| *pending == sequence;

        match timeout {
            Some(to) => {
                let (_guard, toe) = self.condvar.wait_timeout_while(cur, to, cond).unwrap();
                if toe.timed_out() {
                    return Err(());
                } else {
                    return Ok(());
                }
            }
            None => {
                let _guard = self.condvar.wait_while(cur, cond).unwrap();
                return Ok(());
            }
        };
    }

    fn update_next(&self, sequence: usize) {
        *self.seq.lock().unwrap() = sequence;
        self.condvar.notify_all();
    }
}
