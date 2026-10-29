//! Rules, PGN and reversible main-line history.
use std::ops::ControlFlow;
use pgn_reader::{RawTag, Reader, SanPlus, Visitor};
use shakmaty::{fen::Fen, san::SanPlus as WrittenSan, uci::UciMove, CastlingMode, Chess, EnPassantMode, Position, Role, Square};

#[derive(Clone, Debug)]
pub struct Game {
    pub position: Chess,
    initial: Chess,
    history: Vec<(Chess, String, UciMove)>,
}

impl Default for Game {
    fn default() -> Self {
        let initial = Chess::default();
        Self { position: initial.clone(), initial, history: Vec::new() }
    }
}

impl Game {
    pub fn fen(&self) -> String { Fen::from_position(self.position.clone(), EnPassantMode::Legal).to_string() }
    pub fn last_move(&self) -> Option<UciMove> { self.history.last().map(|entry| entry.2) }
    pub fn moves(&self) -> Vec<String> { self.history.iter().map(|entry| entry.1.clone()).collect() }
    pub fn legal_targets(&self, from: Square) -> Vec<Square> {
        self.position.legal_moves().into_iter().filter_map(|m| match UciMove::from_standard(m) {
            UciMove::Normal { from: f, to, .. } if f == from => Some(to),
            _ => None,
        }).collect()
    }
    pub fn play(&mut self, from: Square, to: Square, promotion: Role) -> Result<(), String> {
        let legal = self.position.legal_moves().into_iter().find(|m| match UciMove::from_standard(*m) {
            UciMove::Normal { from: f, to: t, promotion: p } => f == from && t == to && p.is_none_or(|role| role == promotion),
            _ => false,
        }).ok_or_else(|| "Illegal move".to_owned())?;
        self.apply(legal);
        Ok(())
    }
    pub fn play_uci(&mut self, text: &str) -> Result<(), String> {
        let uci: UciMove = text.parse().map_err(|e| format!("Invalid UCI move: {e}"))?;
        let m = uci.to_move(&self.position).map_err(|e| format!("Illegal UCI move: {e}"))?;
        self.apply(m);
        Ok(())
    }
    fn apply(&mut self, m: shakmaty::Move) {
        let old = self.position.clone();
        let san = WrittenSan::from_move_and_play_unchecked(&mut self.position, m).to_string();
        self.history.push((old, san, UciMove::from_standard(m)));
    }
    pub fn undo(&mut self) -> bool {
        if let Some((position, _, _)) = self.history.pop() { self.position = position; true } else { false }
    }
    pub fn reset(&mut self) { self.position = self.initial.clone(); self.history.clear(); }
    pub fn to_pgn(&self) -> String {
        let mut out = String::from("[Event \"Ferrum Chess\"]\n");
        if self.initial != Chess::default() {
            let fen = Fen::from_position(self.initial.clone(), EnPassantMode::Legal);
            out.push_str(&format!("[SetUp \"1\"]\n[FEN \"{fen}\"]\n"));
        }
        let outcome = self.position.outcome().to_string();
        out.push_str(&format!("[Result \"{outcome}\"]\n\n"));
        let mut position = self.initial.clone();
        for (index, (_, san, uci)) in self.history.iter().enumerate() {
            if position.turn().is_white() { out.push_str(&format!("{}. ", position.fullmoves())); }
            else if index == 0 { out.push_str(&format!("{}... ", position.fullmoves())); }
            out.push_str(san); out.push(' ');
            if let Ok(m) = uci.to_move(&position) { position.play_unchecked(m); }
        }
        out.push_str(&outcome); out.push('\n'); out
    }
    pub fn from_pgn(bytes: &[u8]) -> Result<Self, String> {
        let mut reader = Reader::new(bytes);
        reader.read_game(&mut PgnVisitor).map_err(|e| e.to_string())?.ok_or_else(|| "PGN contains no game".to_owned())?
    }
}

struct PgnVisitor;
impl Visitor for PgnVisitor {
    type Tags = Option<Chess>;
    type Movetext = Game;
    type Output = Result<Game, String>;
    fn begin_tags(&mut self) -> ControlFlow<Self::Output, Self::Tags> { ControlFlow::Continue(None) }
    fn tag(&mut self, tags: &mut Self::Tags, name: &[u8], value: RawTag<'_>) -> ControlFlow<Self::Output> {
        if name == b"FEN" {
            let result = Fen::from_ascii(value.as_bytes()).map_err(|e| e.to_string())
                .and_then(|fen| fen.into_position(CastlingMode::Standard).map_err(|e| e.to_string()));
            match result { Ok(position) => *tags = Some(position), Err(e) => return ControlFlow::Break(Err(e)) }
        }
        ControlFlow::Continue(())
    }
    fn begin_movetext(&mut self, tags: Self::Tags) -> ControlFlow<Self::Output, Self::Movetext> {
        let initial = tags.unwrap_or_default();
        ControlFlow::Continue(Game { position: initial.clone(), initial, history: Vec::new() })
    }
    fn san(&mut self, game: &mut Game, san: SanPlus) -> ControlFlow<Self::Output> {
        match san.san.to_move(&game.position) {
            Ok(m) => { game.apply(m); ControlFlow::Continue(()) }
            Err(e) => ControlFlow::Break(Err(format!("Illegal PGN move {san}: {e}"))),
        }
    }
    fn end_game(&mut self, game: Game) -> Self::Output { Ok(game) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn moves_and_undo() {
        let mut game = Game::default();
        assert!(game.play_uci("e2e5").is_err());
        game.play_uci("e2e4").expect("legal move");
        assert_eq!(game.moves(), ["e4"]);
        assert!(game.undo());
        assert_eq!(game.fen(), Game::default().fen());
    }
    #[test] fn pgn_round_trip() {
        let mut game = Game::default();
        for m in ["e2e4", "e7e5", "g1f3"] { game.play_uci(m).expect("legal move"); }
        let parsed = Game::from_pgn(game.to_pgn().as_bytes()).expect("valid PGN");
        assert_eq!(game.fen(), parsed.fen());
        assert_eq!(game.moves(), parsed.moves());
    }
}
