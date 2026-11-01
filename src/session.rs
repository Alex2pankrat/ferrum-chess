//! User-initiated, local snapshots of unfinished games. No remote content is used.
use crate::{archive, game::Game};
use cozy_chess::Color;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    Stockfish,
    Local,
}
#[derive(Clone, Debug)]
pub struct Session {
    pub game: Game,
    pub mode: GameMode,
    pub player: Color,
    pub flipped: bool,
    pub depth: u32,
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub timestamp_ms: u64,
    pub session: Session,
}
pub fn default_directory() -> Option<PathBuf> {
    archive::default_directory()?
        .parent()
        .map(|p| p.join("sessions"))
}
pub fn save(dir: &Path, session: &Session) -> Result<(), String> {
    if session.game.is_game_over() {
        return Err("Only unfinished games can be saved as sessions".into());
    }
    if !(1..=32).contains(&session.depth) {
        return Err("Invalid engine depth".into());
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    let timestamp_ms = u64::try_from(timestamp.as_millis()).map_err(|e| e.to_string())?;
    let mode = match session.mode {
        GameMode::Stockfish => "Stockfish",
        GameMode::Local => "Local",
    };
    let player = if session.player == Color::White {
        "White"
    } else {
        "Black"
    };
    let data = serde_json::to_vec(&serde_json::json!({"format":1, "timestamp_ms":timestamp_ms, "pgn":session.game.to_pgn(), "mode":mode, "player":player, "flipped":session.flipped, "depth":session.depth})).map_err(|e| e.to_string())?;
    fs::create_dir_all(dir).map_err(|e| format!("Cannot create session directory: {e}"))?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "{}-{}-{}",
        timestamp.as_nanos(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let temp = dir.join(format!("{name}.tmp"));
    let target = dir.join(format!("{name}.json"));
    let result = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        f.write_all(&data)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&temp, target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result.map_err(|e| format!("Cannot save session: {e}"))
}
fn parse(data: &[u8]) -> Option<Entry> {
    let json: serde_json::Value = serde_json::from_slice(data).ok()?;
    if json["format"].as_u64()? != 1 {
        return None;
    }
    let timestamp_ms = json["timestamp_ms"].as_u64()?;
    let mode = match json["mode"].as_str()? {
        "Stockfish" => GameMode::Stockfish,
        "Local" => GameMode::Local,
        _ => return None,
    };
    let player = match json["player"].as_str()? {
        "White" => Color::White,
        "Black" => Color::Black,
        _ => return None,
    };
    let flipped = json["flipped"].as_bool()?;
    let depth = u32::try_from(json["depth"].as_u64()?).ok()?;
    if !(1..=32).contains(&depth) {
        return None;
    }
    let game = Game::from_pgn(json["pgn"].as_str()?.as_bytes()).ok()?;
    if game.is_game_over() {
        return None;
    }
    Some(Entry {
        timestamp_ms,
        session: Session {
            game,
            mode,
            player,
            flipped,
            depth,
        },
    })
}
pub fn load(dir: &Path) -> Result<(Vec<Entry>, usize), String> {
    if !dir.exists() {
        return Ok((Vec::new(), 0));
    }
    let files = fs::read_dir(dir).map_err(|e| format!("Cannot read saved sessions: {e}"))?;
    let mut result = Vec::new();
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
        let entry = fs::metadata(&path)
            .ok()
            .filter(|m| m.len() <= 2_000_000)
            .and_then(|_| fs::read(path).ok())
            .and_then(|data| parse(&data));
        if let Some(entry) = entry {
            result.push(entry);
        } else {
            skipped += 1;
        }
    }
    result.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp_ms));
    Ok((result, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sessions_preserve_moves_settings_and_ignore_corruption() {
        let dir = std::env::temp_dir().join(format!(
            "ferrum-session-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut game = Game::default();
        game.play_uci("e2e4").unwrap();
        let session = Session {
            game: game.clone(),
            mode: GameMode::Local,
            player: Color::Black,
            flipped: true,
            depth: 8,
        };
        save(&dir, &session).unwrap();
        save(&dir, &session).unwrap();
        fs::write(dir.join("bad.json"), b"broken").unwrap();
        let (entries, skipped) = load(&dir).unwrap();
        assert_eq!((entries.len(), skipped), (2, 1));
        let restored = &entries[0].session;
        assert_eq!(restored.game.fen(), game.fen());
        assert_eq!(restored.game.moves(), ["e4"]);
        assert_eq!(restored.mode, GameMode::Local);
        assert_eq!(restored.player, Color::Black);
        assert!(restored.flipped);
        assert_eq!(restored.depth, 8);
        game.resign(Color::White).unwrap();
        assert!(save(&dir, &Session { game, ..session }).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_invalid_and_completed_sessions() {
        assert!(parse(br#"{"format":1,"timestamp_ms":0,"mode":"Local","player":"White","depth":8,"flipped":false,"pgn":"1. e4 1-0"}"#).is_none());
        assert!(parse(br#"{"format":1,"timestamp_ms":0,"mode":"Local","player":"White","depth":200,"flipped":false,"pgn":"*"}"#).is_none());
    }
}
