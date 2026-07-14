#![no_main]
use libfuzzer_sys::fuzz_target;

// Property: arbitrary base/delta bytes never panic apply_delta. Split the input
// down the middle so the fuzzer can vary both the base and the delta stream.
fuzz_target!(|data: &[u8]| {
    let (base, delta) = data.split_at(data.len() / 2);
    let _ = ivaldi::pack::apply_delta(base, delta);
});
