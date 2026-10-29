//! Standard-chess game history and a deliberately small, main-line PGN reader.
use cozy_chess::{
    util::{display_san_move, display_uci_move, parse_san_move, parse_uci_move},
    Board, Color, GameStatus, Move, Piece, Square,
};

#[derive(Clone, Debug)]
struct Tags {
    event: String,
    site: String,
    white: String,
    black: String,
    result: String,
}

impl Default for Tags {
    fn default() -> Self {
        Self {
            event: "Ferrum Chess".into(),
            site: "?".into(),
            white: "?".into(),
            black: "?".into(),
            result: "*".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Game {
    pub position: Board,
    initial: Board,
    history: Vec<(Board, String, Move)>,
    tags: Tags,
}

impl Default for Game {
    fn default() -> Self {
        let initial = Board::default();
        Self {
            position: initial.clone(),
            initial,
            history: Vec::new(),
            tags: Tags::default(),
        }
    }
}

impl Game {
    pub fn fen(&self) -> String {
        self.position.to_string()
    }
    pub fn last_move(&self) -> Option<(Square, Square)> {
        self.history.last().and_then(|(board, _, mv)| {
            let uci = display_uci_move(board, *mv).to_string();
            Some((uci.get(..2)?.parse().ok()?, uci.get(2..4)?.parse().ok()?))
        })
    }
    pub fn moves(&self) -> Vec<String> {
        self.history.iter().map(|entry| entry.1.clone()).collect()
    }
    /// Return a copy at a prior ply without changing the original game.
    pub fn at_ply(&self, ply: usize) -> Option<Self> {
        if ply > self.history.len() {
            return None;
        }
        let mut game = self.clone();
        while game.history.len() > ply {
            game.undo();
        }
        Some(game)
    }
    /// Number each move from its actual starting position, including black-to-move FENs.
    pub fn numbered_moves(&self) -> Vec<String> {
        self.history
            .iter()
            .map(|(board, san, _)| {
                let dots = if board.side_to_move() == Color::White {
                    "."
                } else {
                    "..."
                };
                format!("{}{dots} {san}", board.fullmove_number())
            })
            .collect()
    }
    pub fn legal_targets(&self, from: Square) -> Vec<Square> {
        let mut targets = Vec::new();
        self.position.generate_moves_for(from.bitboard(), |moves| {
            for mv in moves {
                let uci = display_uci_move(&self.position, mv).to_string();
                if let Some(to) = uci
                    .get(2..4)
                    .and_then(|s| s.parse().ok())
                    .filter(|to| !targets.contains(to))
                {
                    targets.push(to);
                }
            }
            false
        });
        targets
    }
    pub fn play(&mut self, from: Square, to: Square, promotion: Piece) -> Result<(), String> {
        let mut candidate = None;
        let target = to.to_string();
        self.position.generate_moves_for(from.bitboard(), |moves| {
            for mv in moves {
                let uci = display_uci_move(&self.position, mv).to_string();
                if uci.get(2..4) == Some(target.as_str())
                    && mv.promotion.is_none_or(|p| p == promotion)
                {
                    candidate = Some(mv);
                    return true;
                }
            }
            false
        });
        self.apply(candidate.ok_or_else(|| "Illegal move".to_owned())?)
    }
    pub fn play_uci(&mut self, text: &str) -> Result<(), String> {
        if !text.is_ascii()
            || !(text.len() == 4 || text.len() == 5)
            || (text.len() == 5 && !matches!(text.as_bytes()[4], b'q' | b'r' | b'b' | b'n'))
        {
            return Err("Invalid UCI move".into());
        }
        let mv =
            parse_uci_move(&self.position, text).map_err(|e| format!("Invalid UCI move: {e}"))?;
        self.apply(mv)
    }
    fn apply(&mut self, mv: Move) -> Result<(), String> {
        if !self.position.is_legal(mv) {
            return Err("Illegal move".into());
        }
        // cozy-chess's SAN formatter requires a legal move, checked above.
        let san = display_san_move(&self.position, mv).to_string();
        let old = self.position.clone();
        self.position.try_play(mv).map_err(|e| e.to_string())?;
        self.history.push((old, san, mv));
        self.tags.result = "*".into();
        Ok(())
    }
    pub fn undo(&mut self) -> bool {
        if let Some((position, _, _)) = self.history.pop() {
            self.position = position;
            self.tags.result = "*".into();
            true
        } else {
            false
        }
    }
    pub fn reset(&mut self) {
        self.position = self.initial.clone();
        self.history.clear();
        self.tags.result = "*".into();
    }
    pub fn result(&self) -> &str {
        match self.position.status() {
            GameStatus::Won if self.position.side_to_move() == Color::White => "0-1",
            GameStatus::Won => "1-0",
            GameStatus::Drawn => "1/2-1/2",
            GameStatus::Ongoing => &self.tags.result,
        }
    }
    pub fn is_game_over(&self) -> bool {
        self.position.status() != GameStatus::Ongoing
    }
    pub fn to_pgn(&self) -> String {
        fn escape(value: &str) -> String {
            value.replace('\\', "\\\\").replace('"', "\\\"")
        }
        let mut out = String::new();
        for (name, value) in [
            ("Event", self.tags.event.as_str()),
            ("Site", self.tags.site.as_str()),
            ("White", self.tags.white.as_str()),
            ("Black", self.tags.black.as_str()),
            ("Result", self.result()),
        ] {
            out.push_str(&format!("[{name} \"{}\"]\n", escape(value)));
        }
        if self.initial != Board::default() {
            out.push_str(&format!("[SetUp \"1\"]\n[FEN \"{}\"]\n", self.initial));
        }
        out.push('\n');
        for (index, (board, san, _)) in self.history.iter().enumerate() {
            if board.side_to_move() == Color::White {
                out.push_str(&format!("{}. ", board.fullmove_number()));
            } else if index == 0 {
                out.push_str(&format!("{}... ", board.fullmove_number()));
            }
            out.push_str(san);
            out.push(' ');
        }
        out.push_str(self.result());
        out.push('\n');
        out
    }
    pub fn from_pgn(bytes: &[u8]) -> Result<Self, String> {
        let input = std::str::from_utf8(bytes).map_err(|e| format!("PGN must be UTF-8: {e}"))?;
        let input = input.trim_start_matches('\u{feff}');
        if input.trim().is_empty() {
            return Err("PGN contains no game".into());
        }
        let mut game = Self::default();
        let mut fen = None;
        let mut tag_result = None;
        let mut lines = input.lines().peekable();
        while let Some(line) = lines.peek().copied() {
            let line = line.trim();
            if line.is_empty() {
                lines.next();
                continue;
            }
            if !line.starts_with('[') {
                break;
            }
            let (name, value) = parse_tag(line)?;
            match name.as_str() {
                "Event" => game.tags.event = value,
                "Site" => game.tags.site = value,
                "White" => game.tags.white = value,
                "Black" => game.tags.black = value,
                "Result" => {
                    validate_result(&value)?;
                    tag_result = Some(value);
                }
                "FEN" => fen = Some(value),
                "Variant" if value != "Standard" && value != "Chess" => {
                    return Err("Only standard chess is supported".into())
                }
                _ => {}
            }
            lines.next();
        }
        if let Some(fen) = fen {
            let board = Board::from_fen(&fen, false).map_err(|e| format!("Invalid FEN: {e}"))?;
            game.initial = board.clone();
            game.position = board;
        }
        let movetext = lines.collect::<Vec<_>>().join("\n");
        let tokens = tokenize(&movetext)?;
        let mut movetext_result = None;
        for token in tokens {
            if token.starts_with('$') {
                continue;
            }
            let digits = token.bytes().take_while(u8::is_ascii_digit).count();
            let token = if digits > 0 && token.as_bytes().get(digits) == Some(&b'.') {
                token[digits..].trim_start_matches('.')
            } else {
                token.as_str()
            };
            if token.is_empty() {
                continue;
            }
            if matches!(token, "*" | "1-0" | "0-1" | "1/2-1/2") {
                if movetext_result.is_some() {
                    return Err("Multiple PGN results or games".into());
                }
                movetext_result = Some(token.to_owned());
                continue;
            }
            if movetext_result.is_some() {
                return Err("Unexpected text after PGN result".into());
            }
            let san = token.trim_end_matches(['!', '?']);
            let mv = parse_san_move(&game.position, san)
                .map_err(|_| format!("Invalid or illegal SAN: {token}"))?;
            game.apply(mv)?;
        }
        if tag_result
            .as_ref()
            .zip(movetext_result.as_ref())
            .is_some_and(|(tag, text)| tag != text)
        {
            return Err("PGN result tag and movetext disagree".into());
        }
        game.tags.result = movetext_result.or(tag_result).unwrap_or_else(|| "*".into());
        Ok(game)
    }
}

fn validate_result(value: &str) -> Result<(), String> {
    if matches!(value, "*" | "1-0" | "0-1" | "1/2-1/2") {
        Ok(())
    } else {
        Err("Invalid PGN result".into())
    }
}

fn parse_tag(line: &str) -> Result<(String, String), String> {
    let inner = line
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or("Invalid PGN tag")?
        .trim();
    let split = inner.find(char::is_whitespace).ok_or("Invalid PGN tag")?;
    let name = &inner[..split];
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Invalid PGN tag name".into());
    }
    let quoted = inner[split..].trim();
    let quoted = quoted
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or("Invalid PGN tag value")?;
    let mut value = String::new();
    let mut chars = quoted.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some('\\') => value.push('\\'),
                Some('"') => value.push('"'),
                _ => return Err("Invalid PGN tag escape".into()),
            },
            '"' => return Err("Unescaped quote in PGN tag".into()),
            _ => value.push(ch),
        }
    }
    Ok((name.into(), value))
}

fn tokenize(text: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut comment = false;
    let mut line_comment = false;
    let mut variation = 0usize;
    for ch in text.chars() {
        if line_comment {
            if ch == '\n' {
                line_comment = false;
            }
            continue;
        }
        if comment {
            if ch == '}' {
                comment = false;
            }
            continue;
        }
        match ch {
            '{' => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
                comment = true;
            }
            ';' => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
                line_comment = true;
            }
            '(' => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
                variation += 1;
            }
            ')' => {
                if variation == 0 {
                    return Err("Unmatched PGN variation".into());
                }
                variation -= 1;
            }
            _ if variation > 0 => {}
            c if c.is_whitespace() => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            _ => token.push(ch),
        }
    }
    if comment || variation != 0 {
        return Err("Unclosed PGN comment or variation".into());
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    Ok(tokens)
}
