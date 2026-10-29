//! Blocking UCI adapter. Call from a worker thread, never from the UI thread.
use std::path::PathBuf;
use stockfish::Stockfish;

#[derive(Clone, Debug)]
pub struct Analysis {
    pub best_move: String,
    pub score: String,
    pub depth: u32,
}

fn engine_path() -> PathBuf {
    if let Some(path) = std::env::var_os("FERRUM_STOCKFISH") { return path.into(); }
    let executable = if cfg!(windows) { "stockfish.exe" } else { "stockfish" };
    if let Ok(path) = std::env::current_exe() {
        let adjacent = path.with_file_name(executable);
        if adjacent.is_file() { return adjacent; }
    }
    executable.into()
}

pub fn analyze(fen: String, depth: u32) -> Result<Analysis, String> {
    let path = engine_path();
    let path = path.to_str().ok_or_else(|| "Stockfish path is not valid Unicode".to_owned())?;
    let mut engine = Stockfish::new(path).map_err(|e| format!("Cannot start Stockfish ({path}): {e}"))?;
    engine.setup_for_new_game().map_err(|e| format!("Stockfish setup: {e}"))?;
    engine.set_fen_position(&fen).map_err(|e| format!("Stockfish position: {e}"))?;
    engine.set_depth(depth);
    let result = engine.go().map_err(|e| format!("Stockfish search: {e}"))?;
    Ok(Analysis { best_move: result.best_move().clone(), score: result.eval().to_string(), depth: result.depth() })
}
