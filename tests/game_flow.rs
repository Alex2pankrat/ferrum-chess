use ferrum_chess::game::Game;
use shakmaty::{Position, Square};

#[test]
fn import_export_and_undo_main_line() {
    let source = b"[Event \"Test\"]\n\n1. e4 e5 2. Nf3 Nc6 *";
    let mut game = Game::from_pgn(source).expect("PGN must load");
    assert_eq!(game.moves(), ["e4", "e5", "Nf3", "Nc6"]);
    assert!(game.position.board().piece_at(Square::C6).is_some());
    let copy = Game::from_pgn(game.to_pgn().as_bytes()).expect("PGN must round-trip");
    assert_eq!(game.fen(), copy.fen());
    assert!(game.undo());
    assert!(game.position.board().piece_at(Square::C6).is_none());
    assert_eq!(game.position.turn(), shakmaty::Color::Black);
}

#[test]
fn stockfish_returns_a_legal_move_when_configured() {
    if std::env::var_os("FERRUM_STOCKFISH").is_none() {
        return;
    }
    let mut game = Game::default();
    game.play_uci("e2e4").expect("opening move");
    let analysis = ferrum_chess::engine::analyze(game.fen(), 4).expect("engine search");
    game.play_uci(&analysis.best_move)
        .expect("engine must return a legal move");
}
