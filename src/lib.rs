#![feature(const_generics, maybe_uninit_uninit_array)]
#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![cfg_attr(test, feature(box_syntax))]

pub mod queue;
pub mod timing_wheel;
