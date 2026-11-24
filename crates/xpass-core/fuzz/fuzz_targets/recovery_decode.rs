#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| xpass_core::fuzzing::recovery_decode(data));
