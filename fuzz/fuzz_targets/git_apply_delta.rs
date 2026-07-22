#![no_main]
use libfuzzer_sys::fuzz_target;

// Property: arbitrary base/delta bytes never panic git_remote::apply_delta
// (the git-pack delta decoder, distinct from pack::apply_delta). Split the
// input down the middle so the fuzzer can vary both streams.
fuzz_target!(|data: &[u8]| {
    let (base, delta) = data.split_at(data.len() / 2);
    let _ = ivaldi::git_remote::apply_delta(base, delta);
});
