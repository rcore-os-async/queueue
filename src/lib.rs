#![feature(const_generics)]
#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![cfg_attr(test, feature(box_syntax))]

mod sequencer;
mod slot;
pub mod queue;
