#![no_main]
use libfuzzer_sys::fuzz_target;

// Property: arbitrary bytes never panic, over-allocate, or hang
// parse_packfile — the decoder for packs received from any git host (or a
// MITM'd anonymous clone). Counts, size varints, and zlib streams are all
// attacker-controlled here.
fuzz_target!(|data: &[u8]| {
    let _ = ivaldi::git_remote::parse_packfile(data);
});
