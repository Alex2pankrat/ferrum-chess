#![cfg_attr(windows, windows_subsystem = "windows")]

mod preferences;
mod ui;

use ferrum_chess::{engine, game::Game};
use std::{ffi::OsStr, process::ExitCode};

// Exercise the same rules and UCI adapter as the UI without requiring a desktop.
fn self_test() -> Result<(), String> {
    let mut game = Game::default();
    game.play_uci("e2e4")?;
    let analysis = engine::analyze(game.fen(), 4)?;
    game.play_uci(&analysis.best_move)?;
    let restored = Game::from_pgn(game.to_pgn().as_bytes())?;
    if restored.fen() != game.fen() {
        return Err("PGN round-trip produced a different position".into());
    }
    println!("Self-test passed: legal Stockfish reply and PGN round-trip");
    Ok(())
}

fn main() -> ExitCode {
    if std::env::args_os().nth(1).as_deref() == Some(OsStr::new("--self-test")) {
        return match self_test() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Self-test failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    match ui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Cannot start the application: {error}");
            ExitCode::FAILURE
        }
    }
}
