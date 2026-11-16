//! PTY benchmark (Windows): shell startup time and output throughput through ConPTY.
//! `cargo run --release -p agentdeck-core --example pty_bench` (bundled ConPTY), then again with
//! `AGENTDECK_SYSTEM_CONPTY=1` (Windows' own) to compare.

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const RUNS: usize = 7;

/// Runs `argv` in a fresh PTY until it exits and the output is drained: (elapsed, bytes read).
fn run(argv: &[&str]) -> (Duration, usize) {
    let pair = native_pty_system()
        .openpty(PtySize { rows: 50, cols: 200, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");
    let mut cmd = CommandBuilder::new(argv[0]);
    cmd.args(&argv[1..]);
    let start = Instant::now();
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        let mut total = 0;
        while let Ok(n @ 1..) = reader.read(&mut buf) {
            total += n;
            // Answer the queries a terminal answers (cursor position, device attributes).
            let chunk = &buf[..n];
            if chunk.windows(4).any(|w| w == b"\x1b[6n") {
                let _ = writer.write_all(b"\x1b[1;1R");
            }
            if chunk.windows(3).any(|w| w == b"\x1b[c") || chunk.windows(4).any(|w| w == b"\x1b[0c") {
                let _ = writer.write_all(b"\x1b[?6c");
            }
        }
        let _ = tx.send(total);
    });
    child.wait().expect("wait");
    // ConPTY keeps the pipe open after the child exits: closing it flushes and ends the reader.
    drop(pair.master);
    let bytes = rx.recv_timeout(Duration::from_secs(60)).expect("reader EOF");
    (start.elapsed(), bytes)
}

/// Keystroke echo latency: type one character into an interactive cmd, time until it comes back.
fn echo_latency() -> Duration {
    let pair = native_pty_system()
        .openpty(PtySize { rows: 50, cols: 200, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");
    let mut child = pair.slave.spawn_command(CommandBuilder::new("cmd")).expect("spawn");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        while let Ok(n @ 1..) = reader.read(&mut buf) {
            if tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let wait_for = |needle: &[u8], writer: &mut Box<dyn Write + Send>| {
        let mut seen = Vec::new();
        while !seen.windows(needle.len()).any(|w| w == needle) {
            let chunk = rx.recv_timeout(Duration::from_secs(10)).expect("output");
            if chunk.windows(4).any(|w| w == b"\x1b[6n") {
                let _ = writer.write_all(b"\x1b[1;1R");
            }
            seen.extend_from_slice(&chunk);
        }
    };
    wait_for(b">", &mut writer); // prompt
    let mut samples = Vec::new();
    for key in b"abcdefghijklmnopqrstuvwxyz" {
        std::thread::sleep(Duration::from_millis(30));
        let start = Instant::now();
        writer.write_all(&[*key]).unwrap();
        wait_for(&[*key], &mut writer);
        samples.push(start.elapsed());
    }
    let _ = child.kill();
    median(samples)
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

fn bench(label: &str, argv: &[&str]) {
    if which(argv[0]).is_none() {
        println!("{label:<34} (not installed)");
        return;
    }
    run(argv); // warm-up: disk cache, first-load costs
    let samples: Vec<_> = (0..RUNS).map(|_| run(argv).0).collect();
    println!("{label:<34} median {:>6.0} ms", median(samples).as_secs_f64() * 1000.0);
}

fn which(exe: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .flat_map(|dir| [dir.join(exe), dir.join(format!("{exe}.exe"))])
            .find(|p| p.is_file())
    })
}

fn main() {
    #[cfg(all(windows, target_arch = "x86_64"))]
    agentdeck_core::pty::conpty::preload();
    let conpty = if std::env::var_os("AGENTDECK_SYSTEM_CONPTY").is_some() { "Windows" } else { "bundled" };
    println!("ConPTY: {conpty}\n");

    println!("-- startup (spawn -> exit) --");
    bench("cmd /c exit", &["cmd", "/c", "exit"]);
    bench("powershell (5.1) -NoProfile", &["powershell", "-NoLogo", "-NoProfile", "-Command", "exit"]);
    bench("powershell (5.1) with profile", &["powershell", "-NoLogo", "-Command", "exit"]);
    bench("pwsh (7) -NoProfile", &["pwsh", "-NoLogo", "-NoProfile", "-Command", "exit"]);
    bench("pwsh (7) with profile", &["pwsh", "-NoLogo", "-Command", "exit"]);

    println!("\n-- keystroke echo (interactive cmd) --");
    echo_latency();
    let samples: Vec<_> = (0..5).map(|_| echo_latency()).collect();
    println!("{:<34} median {:>6.2} ms", "key -> echo", median(samples).as_secs_f64() * 1000.0);

    println!("\n-- throughput (type a 32 MB text file) --");
    let file = std::env::temp_dir().join("agentdeck-pty-bench.txt");
    let line = format!("{}\r\n", "agentdeck pty bench 0123456789 ".repeat(3));
    let text = line.repeat(32 * 1024 * 1024 / line.len());
    std::fs::write(&file, &text).unwrap();
    let path = file.to_string_lossy().into_owned();
    // cmd's `type` writes line by line; .NET's CopyTo writes 80 KB blocks.
    let copy = format!("[IO.File]::OpenRead('{path}').CopyTo([Console]::OpenStandardOutput())");
    throughput("type (line writes)", &["cmd", "/c", "type", &path], text.len());
    throughput("CopyTo (block writes)", &["powershell", "-NoProfile", "-Command", &copy], text.len());
    let _ = std::fs::remove_file(&file);
}

/// Best of 3 runs (after a warm-up) of a command printing `size` bytes.
fn throughput(label: &str, argv: &[&str], size: usize) {
    run(argv);
    let (best, bytes) = (0..3).map(|_| run(argv)).min().unwrap();
    println!(
        "{label:<34} {:>6.0} ms, {:.1} MB/s in, {:.1} MB/s out of ConPTY",
        best.as_secs_f64() * 1000.0,
        size as f64 / 1e6 / best.as_secs_f64(),
        bytes as f64 / 1e6 / best.as_secs_f64(),
    );
}
