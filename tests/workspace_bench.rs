//! End-to-end filesystem benchmark (opt-in, release mode):
//! cargo test --release --test workspace_bench -- --ignored --nocapture

use std::fs;
use std::process::Command;
use std::time::{Duration, Instant};

fn run(dir: &std::path::Path, args: &[&str]) -> Duration {
    let start = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_ivaldi"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    start.elapsed()
}

#[test]
#[ignore = "filesystem scale benchmark; run explicitly in release mode"]
fn workspace_commands() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    run(root, &["forge"]);
    run(root, &["config", "--set", "user.name", "Benchmark"]);
    run(
        root,
        &["config", "--set", "user.email", "benchmark@example.com"],
    );
    for index in 0..5000 {
        let subdir = root.join(format!("d{}", index / 100));
        fs::create_dir_all(&subdir).unwrap();
        let mut content = vec![b'x'; 16 * 1024];
        content[..8].copy_from_slice(&(index as u64).to_le_bytes());
        fs::write(subdir.join(format!("f{index}")), content).unwrap();
    }
    std::thread::sleep(Duration::from_secs(4));
    println!("initial gather: {:?}", run(root, &["gather"]));
    run(root, &["seal", "benchmark baseline"]);
    let cache = root.join(".ivaldi/workspace-cache-v1");
    if cache.exists() {
        fs::remove_file(cache).unwrap();
    }
    println!("status without file cache: {:?}", run(root, &["status"]));
    println!("status with file cache: {:?}", run(root, &["status"]));
    println!("unchanged gather: {:?}", run(root, &["gather"]));
    fs::write(root.join("d0/f0"), b"edited").unwrap();
    println!("one-file status: {:?}", run(root, &["status"]));
    println!(
        "explicit one-file gather: {:?}",
        run(root, &["gather", "d0/f0"])
    );
}
