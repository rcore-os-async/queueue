#![feature(const_generics)]
#![cfg_attr(not(any(feature = "std", test)), no_std)]

mod sequencer;
mod slot;
pub mod queue;
