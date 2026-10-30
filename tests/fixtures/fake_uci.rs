//! Standalone UCI test process, compiled by the engine unit tests.
use std::{
    io::{self, BufRead, Write},
    thread,
    time::Duration,
};

fn main() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let name = executable
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if name.contains("hang") {
        thread::sleep(Duration::from_secs(30));
        return Ok(());
    }
    let stdin = io::stdin();
    let mut out = io::stdout().lock();
    let mut received_history = false;
    for line in stdin.lock().lines() {
        let line = line?;
        if line == "uci" {
            writeln!(out, "id name Mock\nuciok")?;
        } else if line == "isready" {
            writeln!(out, "readyok")?;
        } else if line.starts_with("position ") {
            received_history = line.ends_with(" moves e2e4");
        } else if line.starts_with("go ") {
            if name.contains("bad") || (name.contains("history") && !received_history) {
                writeln!(out, "info depth 4 score cp 25\nbestmove e7e4")?;
            } else {
                writeln!(out, "info depth 4 score cp 25\nbestmove e7e5")?;
            }
        }
        out.flush()?;
    }
    Ok(())
}
