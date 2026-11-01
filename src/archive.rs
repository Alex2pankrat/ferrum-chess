//! Local, append-only storage for completed game snapshots.
use crate::game::Game;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug)]
pub struct Entry {
    pub timestamp_ms: u64,
    pub mode: String,
    pub pgn: String,
    pub result: String,
    pub plies: usize,
}

pub fn default_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|p| !p.is_empty())
                .map(|p| PathBuf::from(p).join(".local/share"))
        });
    base.map(|p| p.join("FerrumChess").join("games"))
}

/// Each game completion is a new immutable snapshot; never overwrite another game.
pub fn save(dir: &Path, game: &Game, mode: &str) -> Result<(), String> {
    if !game.is_game_over() {
        return Err("Cannot archive an unfinished game".into());
    }
    fs::create_dir_all(dir).map_err(|e| format!("Cannot create game history directory: {e}"))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    let timestamp = u64::try_from(now.as_millis()).map_err(|e| e.to_string())?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let name = format!("{}-{}-{serial}", now.as_nanos(), std::process::id());
    let path = dir.join(format!("{name}.json"));
    let temporary = dir.join(format!("{name}.tmp"));
    let record = serde_json::json!({"format": 1, "timestamp_ms": timestamp, "mode": mode, "pgn": game.to_pgn()});
    let data = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&data)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| format!("Cannot save game history: {e}"))
}

/// UTC timestamp for display, independent of local time zone settings.
pub fn display_date(timestamp_ms: u64) -> String {
    let seconds = timestamp_ms / 1000;
    let z = (seconds / 86_400).min(i64::MAX as u64 - 719_468) as i64 + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    let hour = (seconds % 86_400) / 3600;
    let minute = (seconds % 3600) / 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

/// Return valid games newest-first, and report how many records could not be read.
pub fn load(dir: &Path) -> Result<(Vec<Entry>, usize), String> {
    if !dir.exists() {
        return Ok((Vec::new(), 0));
    }
    let files = fs::read_dir(dir).map_err(|e| format!("Cannot read game history: {e}"))?;
    let mut records = Vec::new();
    let mut skipped = 0;
    for item in files {
        let Ok(item) = item else {
            skipped += 1;
            continue;
        };
        let path = item.path();
        if path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        let record = (|| -> Option<Entry> {
            if fs::metadata(&path).ok()?.len() > 2_000_000 {
                return None;
            }
            let data = fs::read(&path).ok()?;
            let json: serde_json::Value = serde_json::from_slice(&data).ok()?;
            if json["format"].as_u64()? != 1 {
                return None;
            }
            let timestamp_ms = json["timestamp_ms"].as_u64()?;
            let mode = json["mode"].as_str()?.to_owned();
            let pgn = json["pgn"].as_str()?.to_owned();
            let game = Game::from_pgn(pgn.as_bytes()).ok()?;
            if !game.is_game_over() {
                return None;
            }
            Some(Entry {
                timestamp_ms,
                mode,
                result: game.result().to_owned(),
                plies: game.moves().len(),
                pgn,
            })
        })();
        if let Some(record) = record {
            records.push(record);
        } else {
            skipped += 1;
        }
    }
    records.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
    Ok((records, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamp_is_displayed_in_utc() {
        assert_eq!(display_date(0), "1970-01-01 00:00 UTC");
        assert_eq!(display_date(1_704_067_200_000), "2024-01-01 00:00 UTC");
    }
    #[test]
    fn saves_separate_games_and_ignores_corrupt_files() {
        let dir = std::env::temp_dir().join(format!(
            "ferrum-archive-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut game = Game::default();
        assert!(save(&dir, &game, "Local").is_err());
        game.play_uci("e2e4").unwrap();
        game.resign(cozy_chess::Color::Black).unwrap();
        save(&dir, &game, "Local").unwrap();
        save(&dir, &game, "Stockfish").unwrap();
        fs::write(dir.join("broken.json"), b"invalid").unwrap();
        let (games, skipped) = load(&dir).unwrap();
        assert_eq!(games.len(), 2);
        assert_eq!(skipped, 1);
        assert_eq!(games[0].result, "1-0");
        assert_eq!(
            Game::from_pgn(games[0].pgn.as_bytes()).unwrap().moves(),
            ["e4"]
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
