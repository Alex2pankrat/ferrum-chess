//! Direct UCI adapter. All blocking operations belong on a worker thread.
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct Analysis {
    pub best_move: String,
    /// Score from White's perspective, in centipawns or moves to mate.
    pub score: String,
    pub depth: u32,
}

fn engine_path() -> PathBuf {
    if let Some(path) = std::env::var_os("FERRUM_STOCKFISH") {
        return path.into();
    }
    let executable = if cfg!(windows) {
        "stockfish.exe"
    } else {
        "stockfish"
    };
    if let Ok(path) = std::env::current_exe() {
        let adjacent = path.with_file_name(executable);
        if adjacent.is_file() {
            return adjacent;
        }
    }
    executable.into()
}

struct Session {
    child: Child,
    input: ChildStdin,
    lines: Option<Receiver<Result<String, String>>>,
}

impl Session {
    fn start(path: &Path) -> Result<Self, String> {
        let mut command = Command::new(path);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Cannot start Stockfish ({}): {e}", path.display()))?;
        let Some(input) = child.stdin.take() else {
            terminate(&mut child);
            return Err("Stockfish stdin unavailable".into());
        };
        let Some(output) = child.stdout.take() else {
            terminate(&mut child);
            return Err("Stockfish stdout unavailable".into());
        };
        let (sender, receiver) = mpsc::sync_channel(256);
        let reader = thread::Builder::new()
            .name("uci-reader".into())
            .spawn(move || {
                let mut reader = BufReader::new(output);
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {
                            if sender.send(Ok(line)).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            let _ = sender.send(Err(e.to_string()));
                            break;
                        }
                    }
                }
            });
        if let Err(e) = reader {
            terminate(&mut child);
            return Err(format!("Cannot start UCI reader: {e}"));
        }
        Ok(Self {
            child,
            input,
            lines: Some(receiver),
        })
    }
    fn send(&mut self, command: &str) -> Result<(), String> {
        writeln!(self.input, "{command}")
            .and_then(|()| self.input.flush())
            .map_err(|e| format!("Stockfish input error: {e}"))
    }
    fn receive(&self, deadline: Instant, cancel: &AtomicBool) -> Result<String, String> {
        let receiver = self.lines.as_ref().ok_or("Stockfish output unavailable")?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Stockfish search cancelled".into());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Stockfish timed out".into());
            }
            match receiver.recv_timeout(remaining.min(Duration::from_millis(50))) {
                Ok(Ok(line)) => return Ok(line),
                Ok(Err(e)) => return Err(format!("Stockfish output error: {e}")),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("Stockfish exited before completing the request".into())
                }
            }
        }
    }
    fn wait_for(&self, target: &str, cancel: &AtomicBool) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if self.receive(deadline, cancel)?.trim() == target {
                return Ok(());
            }
        }
    }
}

fn terminate(child: &mut Child) {
    // Never wait indefinitely if the OS refuses the termination request.
    if child.kill().is_ok() {
        let _ = child.wait();
    } else {
        let _ = child.try_wait();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Drop the receiver first so a reader blocked on a full channel can exit.
        self.lines.take();
        terminate(&mut self.child);
    }
}

pub fn analyze(fen: String, depth: u32) -> Result<Analysis, String> {
    analyze_with_cancel(fen, depth, Arc::new(AtomicBool::new(false)))
}

pub fn analyze_with_cancel(
    fen: String,
    depth: u32,
    cancel: Arc<AtomicBool>,
) -> Result<Analysis, String> {
    analyze_at(&engine_path(), &fen, depth, &cancel)
}

/// Analyze a game from its initial FEN and validated UCI moves so the engine can detect repetitions.
pub fn analyze_history_with_cancel(
    initial_fen: String,
    moves: Vec<String>,
    depth: u32,
    cancel: Arc<AtomicBool>,
) -> Result<Analysis, String> {
    analyze_history_at(&engine_path(), &initial_fen, &moves, depth, &cancel)
}

fn position_command(fen: &str, moves: &[String]) -> Result<(cozy_chess::Board, String), String> {
    let mut board =
        cozy_chess::Board::from_fen(fen, false).map_err(|e| format!("Invalid position: {e}"))?;
    let mut command = format!("position fen {board}");
    if !moves.is_empty() {
        command.push_str(" moves");
    }
    for text in moves {
        if !text.is_ascii()
            || !(text.len() == 4 || text.len() == 5)
            || (text.len() == 5 && !matches!(text.as_bytes()[4], b'q' | b'r' | b'b' | b'n'))
        {
            return Err("Invalid UCI history move".into());
        }
        let mv = cozy_chess::util::parse_uci_move(&board, text)
            .map_err(|_| "Invalid UCI history move")?;
        if !board.is_legal(mv) {
            return Err("Illegal UCI history move".into());
        }
        board.try_play(mv).map_err(|e| e.to_string())?;
        command.push(' ');
        command.push_str(text);
    }
    Ok((board, command))
}

fn analyze_at(path: &Path, fen: &str, depth: u32, cancel: &AtomicBool) -> Result<Analysis, String> {
    analyze_history_at(path, fen, &[], depth, cancel)
}

fn analyze_history_at(
    path: &Path,
    fen: &str,
    moves: &[String],
    depth: u32,
    cancel: &AtomicBool,
) -> Result<Analysis, String> {
    let (board, position) = position_command(fen, moves)?;
    if !(1..=32).contains(&depth) {
        return Err("Search depth must be between 1 and 32".into());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Stockfish search cancelled".into());
    }
    let mut engine = Session::start(path)?;
    engine.send("uci")?;
    engine.wait_for("uciok", cancel)?;
    engine.send("isready")?;
    engine.wait_for("readyok", cancel)?;
    engine.send(&position)?;
    // Use a time limit as well as depth so a difficult position cannot run forever.
    engine.send(&format!("go depth {depth} movetime 15000"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut score = "unavailable".to_owned();
    let mut actual_depth = 0;
    loop {
        let line = engine.receive(deadline, cancel)?;
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.first() == Some(&"info") {
            if let Some((d, value)) = parse_info(&parts, board.side_to_move()) {
                actual_depth = d;
                score = value;
            }
        } else if parts.first() == Some(&"bestmove") {
            let best = parts
                .get(1)
                .ok_or("Stockfish returned an incomplete bestmove")?;
            if !best.is_ascii()
                || !(best.len() == 4 || best.len() == 5)
                || (best.len() == 5 && !matches!(best.as_bytes()[4], b'q' | b'r' | b'b' | b'n'))
            {
                return Err(format!("Stockfish returned an invalid bestmove: {best}"));
            }
            let mv = cozy_chess::util::parse_uci_move(&board, best)
                .map_err(|_| "Stockfish returned an invalid move")?;
            if !board.is_legal(mv) {
                return Err(format!("Stockfish returned an illegal move: {best}"));
            }
            return Ok(Analysis {
                best_move: (*best).to_owned(),
                score,
                depth: actual_depth,
            });
        }
    }
}

fn parse_info(parts: &[&str], side: cozy_chess::Color) -> Option<(u32, String)> {
    let depth = parts
        .windows(2)
        .find(|w| w[0] == "depth")?
        .get(1)?
        .parse()
        .ok()?;
    let score = parts.windows(3).find(|w| w[0] == "score")?;
    let value: i32 = score[2].parse().ok()?;
    let white_value = if side == cozy_chess::Color::Black {
        value.checked_neg()?
    } else {
        value
    };
    match score[1] {
        "cp" => Some((depth, format!("{white_value} cp"))),
        "mate" => Some((depth, format!("mate {white_value}"))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fake_engine(mode: &str) -> PathBuf {
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        static NEXT_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ferrum-test-{}-{mode}-{id}{suffix}",
            std::process::id()
        ));
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_uci.rs");
        let status = Command::new("rustc")
            .arg("--edition=2021")
            .arg(source)
            .arg("-o")
            .arg(&path)
            .status()
            .expect("compile fake UCI engine");
        assert!(status.success());
        path
    }

    #[test]
    fn uci_position_preserves_history_and_rejects_invalid_input() {
        let fen = cozy_chess::Board::default().to_string();
        let moves = vec!["e2e4".to_owned(), "e7e5".to_owned()];
        let (board, command) = position_command(&fen, &moves).unwrap();
        assert_eq!(board.side_to_move(), cozy_chess::Color::White);
        assert!(command.ends_with(" moves e2e4 e7e5"));
        assert!(position_command(&fen, &["e2e4\nquit".into()]).is_err());
        assert!(position_command(&fen, &["e2e5".into()]).is_err());
    }

    #[test]
    fn protocol_sends_complete_validated_history() {
        let path = fake_engine("history");
        let result = analyze_history_at(
            &path,
            &cozy_chess::Board::default().to_string(),
            &["e2e4".into()],
            4,
            &AtomicBool::new(false),
        )
        .expect("history-aware response");
        assert_eq!(result.best_move, "e7e5");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn protocol_accepts_valid_moves_and_rejects_illegal_moves() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        let good = fake_engine("good");
        let bad = fake_engine("bad");
        let cancel = AtomicBool::new(false);
        let result = analyze_at(&good, fen, 4, &cancel).expect("valid UCI response");
        assert_eq!(result.best_move, "e7e5");
        assert_eq!(result.score, "-25 cp");
        assert!(analyze_at(&bad, fen, 4, &cancel)
            .expect_err("illegal move")
            .contains("illegal"));
        let _ = std::fs::remove_file(good);
        let _ = std::fs::remove_file(bad);
    }

    #[test]
    fn cancellation_terminates_an_unresponsive_child() {
        let path = fake_engine("hang");
        let path_for_worker = path.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let started = Instant::now();
        let worker = thread::spawn(move || {
            analyze_at(
                &path_for_worker,
                &cozy_chess::Board::default().to_string(),
                4,
                &worker_cancel,
            )
        });
        thread::sleep(Duration::from_millis(200));
        cancel.store(true, Ordering::Relaxed);
        assert!(worker
            .join()
            .expect("worker")
            .expect_err("cancelled")
            .contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn handshake_timeout_terminates_unresponsive_child() {
        let path = fake_engine("hang");
        let start = Instant::now();
        let result = analyze_at(
            &path,
            &cozy_chess::Board::default().to_string(),
            4,
            &AtomicBool::new(false),
        );
        assert!(result.expect_err("timeout").contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(15));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn score_is_converted_to_white_perspective() {
        let info = ["info", "depth", "8", "score", "cp", "-24"];
        assert_eq!(
            parse_info(&info, cozy_chess::Color::Black),
            Some((8, "24 cp".into()))
        );
        assert_eq!(
            parse_info(&["info", "string", "hello"], cozy_chess::Color::White),
            None
        );
    }
    #[test]
    fn cancelled_search_does_not_start_a_process() {
        let cancelled = AtomicBool::new(true);
        let result = analyze_at(
            Path::new("missing-engine"),
            &cozy_chess::Board::default().to_string(),
            4,
            &cancelled,
        );
        assert!(result.expect_err("cancelled").contains("cancelled"));
    }
}
