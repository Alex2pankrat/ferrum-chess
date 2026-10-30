//! Approximate, local, per-move review using the same UCI engine as play.
use crate::{
    engine::{self, Analysis},
    game::Game,
};
use cozy_chess::Color;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

#[derive(Clone, Debug)]
pub struct MoveReview {
    pub label: String,
    /// The engine's evaluation and best move before this move was played.
    pub before: Analysis,
    pub classification: &'static str,
    /// Estimated loss from the mover's perspective, only for centipawn scores.
    pub centipawn_loss: Option<i32>,
}

#[derive(Clone, Debug)]
pub struct GameReport {
    pub moves: Vec<MoveReview>,
}

fn centipawns(score: &str) -> Option<i32> {
    let value = score.strip_suffix(" cp")?.parse::<i32>().ok()?;
    // Bound outliers before subtraction; mate scores are intentionally not compared.
    Some(value.clamp(-20_000, 20_000))
}

fn rate_move(
    played_best: bool,
    side: Color,
    before: &str,
    after: Option<&str>,
) -> (&'static str, Option<i32>) {
    if played_best {
        return ("Best", Some(0));
    }
    let Some((before, after)) = centipawns(before).zip(after.and_then(centipawns)) else {
        return ("Unrated", None);
    };
    let loss = match side {
        Color::White => before - after,
        Color::Black => after - before,
    }
    .max(0);
    let label = if loss >= 300 {
        "Blunder"
    } else if loss >= 150 {
        "Mistake"
    } else if loss >= 60 {
        "Inaccuracy"
    } else {
        "Good"
    };
    (label, Some(loss))
}

/// Analyze every position of the main line. Each search is bounded and cancellable.
/// Progress counts positions completed (there are plies + 1 positions).
pub fn analyze_game_with_cancel(
    game: &Game,
    depth: u32,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
) -> Result<GameReport, String> {
    let labels = game.numbered_moves();
    let moves = game.uci_moves();
    if moves.is_empty() {
        return Err("Play or load a game before analyzing it".into());
    }
    let mut evaluations = Vec::with_capacity(moves.len() + 1);
    for ply in 0..=moves.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Game analysis cancelled".into());
        }
        let position = game.at_ply(ply).ok_or("Invalid game history")?;
        let evaluation = if position.is_game_over() {
            None
        } else {
            Some(engine::analyze_history_with_cancel(
                position.initial_fen(),
                position.uci_moves(),
                depth,
                Arc::clone(&cancel),
            )?)
        };
        evaluations.push(evaluation);
        progress.store(ply + 1, Ordering::Relaxed);
    }
    let mut reviews = Vec::with_capacity(moves.len());
    for (index, played) in moves.iter().enumerate() {
        let Some(before) = evaluations[index].as_ref() else {
            return Err("Game contains moves after a terminal position".into());
        };
        let position = game.at_ply(index).ok_or("Invalid game history")?;
        let (classification, loss) = rate_move(
            played == &before.best_move,
            position.position.side_to_move(),
            &before.score,
            evaluations[index + 1].as_ref().map(|e| e.score.as_str()),
        );
        reviews.push(MoveReview {
            label: labels[index].clone(),
            before: before.clone(),
            classification,
            centipawn_loss: loss,
        });
    }
    Ok(GameReport { moves: reviews })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classify_scores_from_the_movers_perspective() {
        assert_eq!(
            rate_move(false, Color::White, "100 cp", Some("-220 cp")),
            ("Blunder", Some(320))
        );
        assert_eq!(
            rate_move(false, Color::Black, "-100 cp", Some("100 cp")),
            ("Mistake", Some(200))
        );
        assert_eq!(
            rate_move(false, Color::White, "mate 3", Some("300 cp")),
            ("Unrated", None)
        );
        assert_eq!(
            rate_move(true, Color::White, "mate 3", None),
            ("Best", Some(0))
        );
        assert_eq!(
            rate_move(false, Color::White, "-2147483648 cp", Some("2147483647 cp")),
            ("Good", Some(0))
        );
    }
    #[test]
    fn empty_and_cancelled_reports_do_not_run_engine() {
        let cancelled = Arc::new(AtomicBool::new(true));
        let progress = Arc::new(AtomicUsize::new(0));
        assert!(analyze_game_with_cancel(
            &Game::default(),
            4,
            Arc::clone(&cancelled),
            Arc::clone(&progress)
        )
        .is_err());
        let mut game = Game::default();
        game.play_uci("e2e4").unwrap();
        assert!(analyze_game_with_cancel(&game, 4, cancelled, progress)
            .unwrap_err()
            .contains("cancelled"));
    }
}
