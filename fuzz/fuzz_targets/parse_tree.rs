#![no_main]
use libfuzzer_sys::fuzz_target;

// Property: arbitrary bytes never panic parse_tree — it returns Err instead.
fuzz_target!(|data: &[u8]| {
    let _ = ivaldi::fsmerkle::parse_tree(data);
});
