#![feature(const_generics, maybe_uninit_uninit_array, internal_uninit_const, const_fn, const_in_array_repeat_expressions)]
#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![cfg_attr(test, feature(box_syntax))]

pub mod queue;
pub mod timing_wheel;
