use cozy_chess::{Color, Piece, Square};
use ferrum_chess::game::Game;

#[test]
fn import_export_and_undo_main_line() {
    let source = b"[Event \"Test\"]\n[Site \"Home\"]\n[White \"Alice\"]\n[Black \"Bob\"]\n[Result \"*\"]\n\n1. e4 e5 2. Nf3 Nc6 *";
    let mut game = Game::from_pgn(source).expect("PGN must load");
    assert_eq!(game.moves(), ["e4", "e5", "Nf3", "Nc6"]);
    assert_eq!(game.position.piece_on(Square::C6), Some(Piece::Knight));
    let saved = game.to_pgn();
    assert!(saved.contains("[White \"Alice\"]"));
    assert!(saved.contains("[Site \"Home\"]"));
    let copy = Game::from_pgn(saved.as_bytes()).expect("PGN must round-trip");
    assert_eq!(game.fen(), copy.fen());
    assert!(game.undo());
    assert_eq!(game.position.piece_on(Square::C6), None);
    assert_eq!(game.position.side_to_move(), Color::Black);
}

#[test]
fn castling_en_passant_and_promotion_roundtrip() {
    let mut game =
        Game::from_pgn(b"[FEN \"r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1\"]\n\n1. O-O O-O-O *")
            .expect("valid castles");
    assert_eq!(game.position.piece_on(Square::G1), Some(Piece::King));
    assert_eq!(game.last_move(), Some((Square::E8, Square::C8)));
    assert_eq!(
        Game::from_pgn(game.to_pgn().as_bytes())
            .expect("round trip")
            .fen(),
        game.fen()
    );
    assert!(game.undo());
    assert!(game.undo());
    assert!(game.legal_targets(Square::E1).contains(&Square::G1));
    game.play(Square::E1, Square::G1, Piece::Queen)
        .expect("standard castle");
    let ep = Game::from_pgn(b"1. e4 a6 2. e5 d5 3. exd6 *").expect("en passant");
    assert_eq!(ep.position.piece_on(Square::D5), None);
    let mut promotion =
        Game::from_pgn(b"[FEN \"4k3/P7/8/8/8/8/8/4K3 w - - 0 1\"]\n\n*").expect("valid FEN");
    promotion
        .play(Square::A7, Square::A8, Piece::Knight)
        .expect("promote");
    assert_eq!(promotion.position.piece_on(Square::A8), Some(Piece::Knight));
    assert_eq!(
        Game::from_pgn(promotion.to_pgn().as_bytes())
            .expect("promotion PGN")
            .fen(),
        promotion.fen()
    );
}

#[test]
fn rejects_invalid_moves_and_result_conflicts() {
    assert!(Game::from_pgn(b"1. e4 e5 2. Ke7 *").is_err());
    assert!(Game::from_pgn(b"[Result \"1-0\"]\n\n1. e4 0-1").is_err());
    assert!(Game::default().play_uci("e2e4garbage").is_err());
    assert!(Game::default().play_uci("e2e4p").is_err());
}

#[test]
fn parses_comments_variations_and_escaped_tags() {
    let game = Game::from_pgn(
        br#"[Event "A \"quoted\" event"]
[Result "1-0"]

1.e4 {comment} e5 (1... c5 (1... e6)) 2.Nf3 $1 Nc6 1-0"#,
    )
    .expect("PGN");
    assert_eq!(game.moves(), ["e4", "e5", "Nf3", "Nc6"]);
    assert_eq!(game.result(), "1-0");
    assert!(game.to_pgn().contains(r#"[Event "A \"quoted\" event"]"#));
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

#[test]
fn checkmate_exports_a_decisive_result() {
    let game = Game::from_pgn(b"1. f3 e5 2. g4 Qh4# 0-1").expect("valid mate");
    assert!(game.is_game_over());
    assert_eq!(game.result(), "0-1");
    assert!(game.to_pgn().contains("[Result \"0-1\"]"));
}

#[test]
fn application_self_test_reports_missing_engine() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ferrum-chess"))
        .arg("--self-test")
        .env(
            "FERRUM_STOCKFISH",
            "nonexistent-ferrum-stockfish-executable",
        )
        .output()
        .expect("start self-test");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Cannot start Stockfish"));
}

#[test]
fn application_self_test_uses_real_engine_when_configured() {
    if std::env::var_os("FERRUM_STOCKFISH").is_none() {
        return;
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ferrum-chess"))
        .arg("--self-test")
        .output()
        .expect("start self-test");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Self-test passed"));
}

#[test]
fn review_positions_preserve_the_original_and_number_black_to_move() {
    let game = Game::from_pgn(b"[FEN \"7k/8/8/8/8/5K2/8/7R b - - 0 22\"]\n\n22... Kg7 23. Kf4 *")
        .expect("valid game");
    assert_eq!(game.numbered_moves(), ["22... Kg7", "23. Kf4"]);
    let start = game.at_ply(0).unwrap();
    assert!(start.moves().is_empty());
    assert_eq!(game.moves(), ["Kg7", "Kf4"]);
    assert!(game.at_ply(3).is_none());
}

#[test]
fn full_game_review_with_stockfish_when_configured() {
    if std::env::var_os("FERRUM_STOCKFISH").is_none() {
        return;
    }
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    let mut game = Game::default();
    game.play_uci("e2e4").unwrap();
    game.play_uci("e7e5").unwrap();
    let progress = Arc::new(AtomicUsize::new(0));
    let report = ferrum_chess::report::analyze_game_with_cancel(
        &game,
        4,
        Arc::new(AtomicBool::new(false)),
        Arc::clone(&progress),
    )
    .expect("complete game review");
    assert_eq!(report.moves.len(), 2);
    assert_eq!(report.moves[0].label, "1. e4");
    assert_eq!(progress.load(Ordering::Relaxed), 3);
}

#[test]
fn repetition_requires_a_claim_at_three_and_is_automatic_at_five() {
    let mut game = Game::default();
    let cycle = ["g1f3", "g8f6", "f3g1", "f6g8"];
    for mv in cycle.iter().cycle().take(8) {
        game.play_uci(mv).unwrap();
    }
    assert_eq!(game.repetition_count(), 3);
    assert!(!game.is_game_over());
    assert!(game.can_claim_repetition());
    let mut claimed = game.clone();
    claimed.claim_repetition().unwrap();
    assert_eq!(claimed.result(), "1/2-1/2");
    assert!(claimed.play_uci("e2e4").is_err());
    assert!(claimed.undo());
    assert_eq!(claimed.moves().len(), 8);
    assert!(!claimed.is_game_over());
    for mv in cycle.iter().cycle().take(8) {
        game.play_uci(mv).unwrap();
    }
    assert_eq!(game.repetition_count(), 5);
    assert!(game.is_game_over());
    assert_eq!(game.result(), "1/2-1/2");
    assert!(game.play_uci("e2e4").is_err());
    assert!(game.undo());
    assert!(!game.is_game_over());
}

#[test]
fn repetition_respects_castling_rights() {
    let mut game = Game::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
    for mv in ["h1h2", "h8h7", "h2h1", "h7h8"] {
        game.play_uci(mv).unwrap();
    }
    assert_eq!(game.repetition_count(), 1);
}

#[test]
fn conservative_dead_material_does_not_declare_live_positions_drawn() {
    for fen in [
        "4k3/8/8/8/8/8/8/4K3 w - - 0 1",
        "4k3/8/8/8/8/8/8/2B1K3 w - - 0 1",
        "4k3/8/8/8/8/4b3/8/2B1K3 w - - 0 1",
    ] {
        let game = Game::from_fen(fen).unwrap();
        assert!(game.is_game_over(), "{fen}");
        assert_eq!(game.result(), "1/2-1/2");
    }
    for fen in [
        "4k3/8/8/8/4b3/8/8/2B1K3 w - - 0 1",
        "4k3/8/8/8/8/8/8/1NN1K3 w - - 0 1",
        "4k3/8/8/8/8/8/P7/4K3 w - - 0 1",
    ] {
        assert!(!Game::from_fen(fen).unwrap().is_game_over(), "{fen}");
    }
}

#[test]
fn manual_result_roundtrips_and_undo_reopens_it() {
    for (side, result) in [(Color::White, "0-1"), (Color::Black, "1-0")] {
        let mut game = Game::default();
        game.play_uci("e2e4").unwrap();
        game.resign(side).unwrap();
        assert_eq!(game.result(), result);
        let mut loaded = Game::from_pgn(game.to_pgn().as_bytes()).unwrap();
        assert!(loaded.is_game_over());
        assert_eq!(loaded.result(), result);
        assert!(loaded.play_uci("e7e5").is_err());
        assert!(loaded.undo());
        assert_eq!(loaded.moves().len(), 1);
        loaded.play_uci("e7e5").unwrap();
    }
    let mut game = Game::default();
    game.agree_draw().unwrap();
    let loaded = Game::from_pgn(game.to_pgn().as_bytes()).unwrap();
    assert!(loaded.is_game_over());
    assert_eq!(loaded.result(), "1/2-1/2");
    assert!(Game::from_pgn(b"1. f3 e5 2. g4 Qh4# 1-0").is_err());
}

#[test]
fn fen_import_rejects_invalid_positions_and_pgn_keeps_starting_fen() {
    assert!(Game::from_fen("not a position").is_err());
    let fen = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
    let mut game = Game::from_fen(fen).unwrap();
    game.play_uci("e1g1").unwrap();
    let loaded = Game::from_pgn(game.to_pgn().as_bytes()).unwrap();
    assert_eq!(loaded.initial_fen(), fen);
    assert_eq!(loaded.fen(), game.fen());
}
