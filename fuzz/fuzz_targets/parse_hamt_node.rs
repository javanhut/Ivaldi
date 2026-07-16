#![no_main]
use libfuzzer_sys::fuzz_target;

// Property: arbitrary bytes never panic parse_node — it returns Err instead.
fuzz_target!(|data: &[u8]| {
    let _ = ivaldi::hamt::parse_node(data);
});
