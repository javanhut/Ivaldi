#![no_main]
use libfuzzer_sys::fuzz_target;

// Property: arbitrary bytes never panic parse_chunk_node — it returns Err
// instead. Chunk nodes arrive verbatim from peers.
fuzz_target!(|data: &[u8]| {
    let _ = ivaldi::filechunk::parse_node(data);
});
