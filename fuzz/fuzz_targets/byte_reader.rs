#![no_main]
use ivaldi::reader::ByteReader;
use libfuzzer_sys::fuzz_target;

// Property: every ByteReader read is bounds-checked — arbitrary bytes and any
// sequence of reads return Err, never panic.
fuzz_target!(|data: &[u8]| {
    let mut r = ByteReader::new(data);
    for _ in 0..8 {
        let _ = r.uvarint();
        let _ = r.varint();
        let _ = r.u8();
        let _ = r.array::<32>();
        let _ = r.string("f");
        if let Ok(n) = r.uvarint() {
            let _ = r.take(n as usize);
        }
    }
    let _ = r.finish();
});
