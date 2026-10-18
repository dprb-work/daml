#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| daml_fuzz::exercise_format(data));
