#![no_main]

use libfuzzer_sys::fuzz_target;

// Protocol: first byte J selects strict JSON; D selects native DAML; remaining bytes are the module.
fuzz_target!(|data: &[u8]| {
    daml_fuzz::exercise_schema(data);
});
