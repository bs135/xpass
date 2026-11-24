#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| xpass_core::fuzzing::unlock_hello(data));
