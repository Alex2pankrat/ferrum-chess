//! Small local-only appearance preferences.
use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BoardTheme {
    #[default]
    Forest,
    Walnut,
    Slate,
}
impl fmt::Display for BoardTheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PieceSet {
    #[default]
    Livius,
    Meridian,
}
impl fmt::Display for PieceSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub board: BoardTheme,
    pub pieces: PieceSet,
    pub coordinates: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            board: BoardTheme::Forest,
            pieces: PieceSet::Livius,
            coordinates: true,
        }
    }
}
impl Preferences {
    fn parse(data: &str) -> Self {
        let mut prefs = Self::default();
        for line in data.lines() {
            match line {
                "board=Forest" => prefs.board = BoardTheme::Forest,
                "board=Walnut" => prefs.board = BoardTheme::Walnut,
                "board=Slate" => prefs.board = BoardTheme::Slate,
                "pieces=Livius" => prefs.pieces = PieceSet::Livius,
                "pieces=Meridian" => prefs.pieces = PieceSet::Meridian,
                "coordinates=true" => prefs.coordinates = true,
                "coordinates=false" => prefs.coordinates = false,
                _ => {}
            }
        }
        prefs
    }
    fn path() -> Option<PathBuf> {
        #[cfg(windows)]
        let base = std::env::var_os("APPDATA").map(PathBuf::from);
        #[cfg(not(windows))]
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|p| !p.is_empty())
                    .map(|p| PathBuf::from(p).join(".config"))
            });
        base.map(|p| p.join("FerrumChess").join("preferences.ini"))
    }
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| fs::read_to_string(p).ok())
            .map_or_else(Self::default, |s| Self::parse(&s))
    }
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("Preferences directory is unavailable")?;
        self.save_to(&path)
            .map_err(|e| format!("Cannot save appearance settings: {e}"))
    }
    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            path,
            format!(
                "board={}\npieces={}\ncoordinates={}\n",
                self.board, self.pieces, self.coordinates
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_round_trip_and_ignore_unknown_values() {
        let prefs = Preferences {
            board: BoardTheme::Walnut,
            pieces: PieceSet::Meridian,
            coordinates: false,
        };
        let path = std::env::temp_dir().join(format!(
            "ferrum-preferences-{}-{}.ini",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        prefs.save_to(&path).unwrap();
        assert_eq!(
            Preferences::parse(&fs::read_to_string(&path).unwrap()),
            prefs
        );
        fs::remove_file(path).unwrap();
        assert_eq!(
            Preferences::parse("board=unknown\npieces=unknown\ncoordinates=invalid\n"),
            Preferences::default()
        );
    }
}
