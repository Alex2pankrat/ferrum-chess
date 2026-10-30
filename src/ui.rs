//! Iced MVU application and interactive board.
use crate::preferences::{BoardTheme, PieceSet, Preferences};
use cozy_chess::{Color as Side, Piece as Role, Square};
use ferrum_chess::{
    engine::{self, Analysis},
    game::Game,
    report::{self, GameReport},
};
use iced::{
    futures::channel::oneshot,
    mouse,
    widget::{
        button,
        canvas::{self, Canvas, Frame, Geometry, Path, Stroke},
        checkbox, column, container, pick_list, responsive, row, scrollable, svg, text, text_input,
    },
    Color, Element, Length, Point, Rectangle, Renderer, Size, Subscription, Task, Theme,
};
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, OnceLock,
    },
};

pub fn run() -> iced::Result {
    iced::application(App::default, App::update, App::view)
        .title("Ferrum Chess")
        .subscription(App::subscription)
        .theme(Theme::custom(
            "Ferrum",
            iced::theme::Palette {
                background: Color::from_rgb8(20, 25, 27),
                text: Color::from_rgb8(235, 238, 229),
                primary: Color::from_rgb8(142, 181, 123),
                success: Color::from_rgb8(142, 181, 123),
                warning: Color::from_rgb8(226, 182, 98),
                danger: Color::from_rgb8(202, 100, 96),
            },
        ))
        .window(iced::window::Settings {
            size: Size::new(1200.0, 800.0),
            min_size: Some(Size::new(520.0, 500.0)),
            icon: iced::window::icon::from_rgba(
                include_bytes!("../assets/ferrum-icon.rgba").to_vec(),
                64,
                64,
            )
            .ok(),
            ..iced::window::Settings::default()
        })
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Play,
    Analysis,
}
impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Play => "Play Stockfish",
                Self::Analysis => "Analysis",
            }
        )
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Player {
    White,
    Black,
}
impl fmt::Display for Player {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl Player {
    fn side(self) -> Side {
        match self {
            Self::White => Side::White,
            Self::Black => Side::Black,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Promotion {
    Queen,
    Rook,
    Bishop,
    Knight,
}
impl fmt::Display for Promotion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl Promotion {
    fn role(self) -> Role {
        match self {
            Self::Queen => Role::Queen,
            Self::Rook => Role::Rook,
            Self::Bishop => Role::Bishop,
            Self::Knight => Role::Knight,
        }
    }
}

struct App {
    game: Game,
    mode: Mode,
    player: Player,
    flipped: bool,
    preferences: Preferences,
    settings_open: bool,
    settings_error: Option<String>,
    pending_promotion: Option<(Square, Square)>,
    review: Option<Game>,
    move_input: String,
    depth: u32,
    selected: Option<Square>,
    analysis: Option<Analysis>,
    game_report: Option<GameReport>,
    report_progress: Option<Arc<AtomicUsize>>,
    busy: bool,
    search_cancel: Option<Arc<AtomicBool>>,
    generation: u64,
    notice: String,
}
impl Default for App {
    fn default() -> Self {
        Self {
            game: Game::default(),
            mode: Mode::Play,
            player: Player::White,
            flipped: false,
            preferences: Preferences::load(),
            settings_open: false,
            settings_error: None,
            pending_promotion: None,
            review: None,
            move_input: String::new(),
            depth: 10,
            selected: None,
            analysis: None,
            game_report: None,
            report_progress: None,
            busy: false,
            search_cancel: None,
            generation: 0,
            notice: "Choose a piece to see legal moves".into(),
        }
    }
}
#[derive(Debug, Clone)]
enum Message {
    Click(Square),
    Drag(Square, Square),
    Mode(Mode),
    Player(Player),
    Depth(u32),
    Promote(Promotion),
    CancelPromotion,
    Appearance(bool),
    BoardTheme(BoardTheme),
    PieceSet(PieceSet),
    Coordinates(bool),
    MoveTextChanged(String),
    SubmitMove,
    Review(usize),
    Flip,
    New,
    Reset,
    Undo,
    Analyze,
    AnalyzeGame,
    CancelAnalysis,
    ReportTick,
    ReportMove(usize),
    Report(u64, Result<GameReport, String>),
    Engine(u64, Result<Analysis, String>),
    Open,
    Save,
    Opened(u64, Result<Option<Box<Game>>, String>),
    Saved(Result<bool, String>),
}

fn worker<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> impl std::future::Future<Output = Result<T, String>> {
    let (send, recv) = oneshot::channel();
    let _ = std::thread::Builder::new()
        .name("ferrum-worker".into())
        .spawn(move || {
            let _ = send.send(job());
        });
    async move {
        recv.await
            .map_err(|_| "Background task terminated unexpectedly".to_owned())
    }
}

impl App {
    fn invalidate(&mut self) {
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        self.report_progress = None;
        self.analysis = None;
        self.selected = None;
        self.pending_promotion = None;
    }
    fn subscription(&self) -> Subscription<Message> {
        if self.report_progress.is_some() {
            iced::time::every(std::time::Duration::from_millis(300)).map(|_| Message::ReportTick)
        } else {
            Subscription::none()
        }
    }
    fn analyze_game(&mut self) -> Task<Message> {
        if self.busy || self.pending_promotion.is_some() {
            self.notice =
                "Finish the current move or wait for the engine before reviewing the game".into();
            return Task::none();
        }
        let game = self.review.clone().unwrap_or_else(|| self.game.clone());
        if game.moves().is_empty() {
            self.notice = "Play or load a game before analyzing it".into();
            return Task::none();
        }
        self.invalidate();
        self.game_report = None;
        self.busy = true;
        self.notice = "Analyzing every position with Stockfish…".into();
        let generation = self.generation;
        let depth = self.depth;
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicUsize::new(0));
        self.search_cancel = Some(Arc::clone(&cancel));
        self.report_progress = Some(Arc::clone(&progress));
        Task::perform(
            worker(move || report::analyze_game_with_cancel(&game, depth, cancel, progress)),
            move |result| Message::Report(generation, result.and_then(std::convert::identity)),
        )
    }
    fn search(&mut self) -> Task<Message> {
        if self.game.is_game_over() {
            self.notice = format!("Game over: {}", self.game.result());
            return Task::none();
        }
        self.busy = true;
        self.notice = "Stockfish is thinking…".into();
        let generation = self.generation;
        let fen = self.game.fen();
        let depth = self.depth;
        let cancel = Arc::new(AtomicBool::new(false));
        self.search_cancel = Some(Arc::clone(&cancel));
        Task::perform(
            worker(move || engine::analyze_with_cancel(fen, depth, cancel)),
            move |result| Message::Engine(generation, result.and_then(std::convert::identity)),
        )
    }
    fn maybe_search(&mut self) -> Task<Message> {
        if self.mode == Mode::Analysis || self.game.position.side_to_move() != self.player.side() {
            self.search()
        } else {
            Task::none()
        }
    }
    fn move_piece(&mut self, from: Square, to: Square) -> Task<Message> {
        if self.busy
            || self.pending_promotion.is_some()
            || self.review.is_some()
            || (self.mode == Mode::Play && self.game.position.side_to_move() != self.player.side())
        {
            return Task::none();
        }
        let promotes = self.game.position.piece_on(from) == Some(Role::Pawn)
            && (to.to_string().ends_with('8') || to.to_string().ends_with('1'))
            && self.game.legal_targets(from).contains(&to);
        if promotes {
            self.pending_promotion = Some((from, to));
            self.settings_open = false;
            self.notice = "Choose a piece for your pawn".into();
            return Task::none();
        }
        self.finish_move(from, to, Role::Queen)
    }
    fn finish_move(&mut self, from: Square, to: Square, promotion: Role) -> Task<Message> {
        match self.game.play(from, to, promotion) {
            Ok(()) => {
                self.review = None;
                self.game_report = None;
                self.invalidate();
                self.notice = format!("{:?} to move", self.game.position.side_to_move());
                self.maybe_search()
            }
            Err(_) => {
                self.selected = (self.game.position.color_on(to)
                    == Some(self.game.position.side_to_move()))
                .then_some(to);
                Task::none()
            }
        }
    }
    fn save_preferences(&mut self) {
        self.settings_error = self.preferences.save().err();
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Click(square) => {
                if self.busy
                    || self.pending_promotion.is_some()
                    || self.review.is_some()
                    || (self.mode == Mode::Play
                        && self.game.position.side_to_move() != self.player.side())
                {
                    return Task::none();
                }
                if let Some(from) = self.selected.filter(|from| *from != square) {
                    return self.move_piece(from, square);
                }
                self.selected = (self.game.position.color_on(square)
                    == Some(self.game.position.side_to_move()))
                .then_some(square);
            }
            Message::Drag(from, to) => return self.move_piece(from, to),
            Message::Mode(mode) => {
                self.mode = mode;
                self.settings_open = false;
                self.invalidate();
                return self.maybe_search();
            }
            Message::Player(player) => {
                self.player = player;
                self.flipped = player == Player::Black;
                self.game = Game::default();
                self.game_report = None;
                self.review = None;
                self.invalidate();
                return self.maybe_search();
            }
            Message::Depth(depth) => {
                self.depth = depth;
                self.invalidate();
                return self.maybe_search();
            }
            Message::Promote(p) => {
                if let Some((from, to)) = self.pending_promotion.take() {
                    return self.finish_move(from, to, p.role());
                }
            }
            Message::CancelPromotion => {
                self.pending_promotion = None;
                self.notice = "Promotion cancelled".into();
            }
            Message::Appearance(open) => self.settings_open = open,
            Message::BoardTheme(theme) => {
                self.preferences.board = theme;
                self.save_preferences();
            }
            Message::PieceSet(set) => {
                self.preferences.pieces = set;
                self.save_preferences();
            }
            Message::Coordinates(value) => {
                self.preferences.coordinates = value;
                self.save_preferences();
            }
            Message::MoveTextChanged(value) => self.move_input = value,
            Message::SubmitMove => {
                if self.busy
                    || self.pending_promotion.is_some()
                    || self.review.is_some()
                    || self.game.is_game_over()
                    || (self.mode == Mode::Play
                        && self.game.position.side_to_move() != self.player.side())
                {
                    self.notice = "Cannot move right now".into();
                    return Task::none();
                }
                let input = self.move_input.trim().to_ascii_lowercase();
                if !input.is_ascii() || !(input.len() == 4 || input.len() == 5) {
                    self.notice = "Enter a move like e2e4 or e7e8q".into();
                    return Task::none();
                }
                let from = input.get(..2).and_then(|v| v.parse::<Square>().ok());
                let to = input.get(2..4).and_then(|v| v.parse::<Square>().ok());
                if let (Some(from), Some(to)) = (from, to) {
                    if input.len() == 4 && self.game.legal_targets(from).contains(&to) {
                        self.move_input.clear();
                        return self.move_piece(from, to);
                    }
                    if input.len() == 5 {
                        match self.game.play_uci(&input) {
                            Ok(()) => {
                                self.move_input.clear();
                                self.game_report = None;
                                self.review = None;
                                self.invalidate();
                                self.notice =
                                    format!("{:?} to move", self.game.position.side_to_move());
                                return self.maybe_search();
                            }
                            Err(error) => self.notice = error,
                        }
                    } else {
                        self.notice = "Illegal move".into();
                    }
                } else {
                    self.notice = "Enter a move like e2e4 or e7e8q".into();
                }
            }
            Message::Review(ply) => {
                let full_game = self.review.clone().unwrap_or_else(|| self.game.clone());
                if let Some(position) = full_game.at_ply(ply) {
                    self.invalidate();
                    let total = full_game.moves().len();
                    self.game = position;
                    self.review = (ply < total).then_some(full_game);
                    self.notice = if self.review.is_some() {
                        format!(
                            "Reviewing move {ply} of {total} — return to the latest move to play"
                        )
                    } else {
                        "Latest position".into()
                    };
                    if self.review.is_none() {
                        return self.maybe_search();
                    }
                }
            }
            Message::Flip => self.flipped = !self.flipped,
            Message::New => {
                self.game = Game::default();
                self.game_report = None;
                self.review = None;
                self.invalidate();
                return self.maybe_search();
            }
            Message::Reset => {
                self.game.reset();
                self.game_report = None;
                self.review = None;
                self.invalidate();
                return self.maybe_search();
            }
            Message::Undo => {
                if self.review.is_some() {
                    return self.update(Message::Review(self.game.moves().len().saturating_sub(1)));
                }
                self.game_report = None;
                self.invalidate();
                if self.game.undo()
                    && self.mode == Mode::Play
                    && self.game.position.side_to_move() != self.player.side()
                {
                    self.game.undo();
                }
                self.notice = "Move undone".into();
                return self.maybe_search();
            }
            Message::Analyze => {
                self.invalidate();
                return self.search();
            }
            Message::AnalyzeGame => return self.analyze_game(),
            Message::CancelAnalysis => {
                self.invalidate();
                self.notice = "Analysis cancelled".into();
            }
            Message::ReportTick => {}
            Message::ReportMove(index) => {
                if let Some(entry) = self
                    .game_report
                    .as_ref()
                    .and_then(|report| report.moves.get(index))
                    .cloned()
                {
                    let task = self.update(Message::Review(index));
                    self.analysis = Some(entry.before);
                    self.notice = format!(
                        "{}: {}. Best move shown on the board.",
                        entry.label, entry.classification
                    );
                    return task;
                }
            }
            Message::Report(generation, result) => {
                if generation != self.generation {
                    return Task::none();
                }
                self.busy = false;
                self.search_cancel = None;
                self.report_progress = None;
                match result {
                    Ok(report) => {
                        self.notice = format!("Game review complete: {} moves", report.moves.len());
                        self.game_report = Some(report);
                    }
                    Err(error) => self.notice = error,
                }
            }
            Message::Engine(generation, result) => {
                if generation != self.generation {
                    return Task::none();
                }
                self.busy = false;
                self.search_cancel = None;
                match result {
                    Ok(analysis) => {
                        self.notice = format!(
                            "Evaluation (White): {} · depth {} · best {}",
                            analysis.score, analysis.depth, analysis.best_move
                        );
                        if self.mode == Mode::Play
                            && self.game.position.side_to_move() != self.player.side()
                            && self.review.is_none()
                        {
                            self.game_report = None;
                            if let Err(e) = self.game.play_uci(&analysis.best_move) {
                                self.notice = e;
                            } else if self.game.is_game_over() {
                                self.notice = format!("Game over: {}", self.game.result());
                            }
                            self.analysis = None;
                        } else {
                            self.analysis = Some(analysis);
                        }
                    }
                    Err(error) => self.notice = error,
                }
            }
            Message::Open => {
                self.invalidate();
                let generation = self.generation;
                return Task::perform(
                    worker(|| {
                        let Some(path) = rfd::FileDialog::new()
                            .add_filter("PGN", &["pgn"])
                            .pick_file()
                        else {
                            return Ok(None);
                        };
                        let data = std::fs::read(path).map_err(|e| e.to_string())?;
                        Game::from_pgn(&data).map(Box::new).map(Some)
                    }),
                    move |result| {
                        Message::Opened(generation, result.and_then(std::convert::identity))
                    },
                );
            }
            Message::Opened(generation, result) => {
                if generation != self.generation {
                    return Task::none();
                }
                match result {
                    Ok(Some(game)) => {
                        self.game = *game;
                        self.game_report = None;
                        self.review = None;
                        self.invalidate();
                        self.notice = "PGN loaded".into();
                        return self.maybe_search();
                    }
                    Ok(None) => {}
                    Err(e) => self.notice = format!("Cannot open PGN: {e}"),
                }
            }
            Message::Save => {
                let pgn = self.review.as_ref().unwrap_or(&self.game).to_pgn();
                return Task::perform(
                    worker(move || {
                        let Some(path) = rfd::FileDialog::new()
                            .add_filter("PGN", &["pgn"])
                            .set_file_name("game.pgn")
                            .save_file()
                        else {
                            return Ok(false);
                        };
                        std::fs::write(path, pgn).map_err(|e| e.to_string())?;
                        Ok(true)
                    }),
                    |result| Message::Saved(result.and_then(std::convert::identity)),
                );
            }
            Message::Saved(result) => match result {
                Ok(true) => self.notice = "PGN saved".into(),
                Ok(false) => {}
                Err(e) => self.notice = format!("Cannot save PGN: {e}"),
            },
        }
        Task::none()
    }
    fn view(&self) -> Element<'_, Message> {
        responsive(move |size| self.layout(size)).into()
    }

    fn layout(&self, size: Size) -> Element<'_, Message> {
        let compact = size.width < 900.0;
        let board_size = if compact {
            (size.width - 48.0).min(720.0)
        } else {
            (size.width - 406.0).min(size.height - 206.0).min(760.0)
        }
        .max(80.0);
        let is_analysis = self.mode == Mode::Analysis;
        let top = row![
            column![
                text("FERRUM").size(20),
                text("CHESS STUDIO").size(10).color(muted()),
            ]
            .spacing(0)
            .width(Length::Fill),
            button("Play")
                .on_press(Message::Mode(Mode::Play))
                .style(if is_analysis {
                    button::secondary
                } else {
                    button::primary
                })
                .padding([9, 18]),
            button("Analysis")
                .on_press(Message::Mode(Mode::Analysis))
                .style(if is_analysis {
                    button::primary
                } else {
                    button::secondary
                })
                .padding([9, 18]),
            button("Appearance")
                .on_press(Message::Appearance(!self.settings_open))
                .style(if self.settings_open {
                    button::primary
                } else {
                    button::secondary
                })
                .padding([9, 14]),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let board = Canvas::new(Board { app: self })
            .width(board_size)
            .height(board_size);
        let board: Element<'_, Message> = if self.pending_promotion.is_some() {
            let side = self.game.position.side_to_move();
            let choice = |label, kind: Promotion| {
                button(
                    column![
                        svg::Svg::new(
                            piece_handle(self.preferences.pieces, side, kind.role()).clone()
                        )
                        .width(42)
                        .height(42),
                        text(label).size(12),
                    ]
                    .align_x(iced::Alignment::Center)
                    .spacing(3),
                )
                .style(button::secondary)
                .padding(6)
                .on_press(Message::Promote(kind))
            };
            let dialog = container(
                column![
                    text("CHOOSE PROMOTION").size(13).color(accent()),
                    row![
                        choice("Queen", Promotion::Queen),
                        choice("Rook", Promotion::Rook),
                        choice("Bishop", Promotion::Bishop),
                        choice("Knight", Promotion::Knight)
                    ]
                    .spacing(4),
                    button("Cancel")
                        .on_press(Message::CancelPromotion)
                        .style(button::text),
                ]
                .spacing(10),
            )
            .padding(16)
            .style(card_style);
            iced::widget::stack![
                board,
                container(dialog).center_x(board_size).center_y(board_size)
            ]
            .into()
        } else {
            board.into()
        };
        let opponent = if self.flipped { "White" } else { "Black" };
        let bottom = if self.flipped { "Black" } else { "White" };
        let board_area = column![
            row![
                text(opponent).size(16),
                text("  •  ").color(muted()),
                text(if is_analysis {
                    "Analysis"
                } else if self.player.side()
                    == if self.flipped {
                        Side::White
                    } else {
                        Side::Black
                    }
                {
                    "You"
                } else {
                    "Stockfish"
                })
                .size(13)
                .color(muted())
            ]
            .align_y(iced::Alignment::Center),
            board,
            row![
                text(bottom).size(16),
                text("  •  ").color(muted()),
                text(if is_analysis {
                    "Analysis"
                } else if self.player.side()
                    == if self.flipped {
                        Side::Black
                    } else {
                        Side::White
                    }
                {
                    "You"
                } else {
                    "Stockfish"
                })
                .size(13)
                .color(muted())
            ]
            .align_y(iced::Alignment::Center),
        ]
        .spacing(8);

        let current_ply = self.game.moves().len();
        let full_game = self.review.as_ref().unwrap_or(&self.game);
        let moves = full_game.numbered_moves();
        let total_moves = moves.len();
        let move_list: Element<'_, Message> = if moves.is_empty() {
            text("Your moves will appear here.")
                .size(14)
                .color(muted())
                .into()
        } else {
            column(moves.into_iter().enumerate().map(|(index, label)| {
                button(text(label).size(14).width(Length::Fill))
                    .style(if current_ply == index + 1 {
                        button::secondary
                    } else {
                        button::text
                    })
                    .padding([5, 8])
                    .width(Length::Fill)
                    .on_press(Message::Review(index + 1))
                    .into()
            }))
            .spacing(2)
            .into()
        };
        let heading = if is_analysis {
            "Analysis board"
        } else {
            "Play Stockfish"
        };
        let mode_description = if is_analysis {
            "Explore a position with the engine"
        } else {
            "A game at your own pace"
        };
        let status = if self.busy {
            "ENGINE THINKING"
        } else if self.review.is_some() {
            "REVIEW"
        } else if self.game.is_game_over() {
            "GAME OVER"
        } else {
            "READY"
        };
        let review_control: Element<'_, Message> = if let Some(progress) = &self.report_progress {
            column![
                text(format!(
                    "Reviewing positions: {} / {}",
                    progress.load(Ordering::Relaxed),
                    total_moves + 1
                ))
                .size(12)
                .color(muted()),
                button("Cancel game review")
                    .on_press(Message::CancelAnalysis)
                    .style(button::secondary),
            ]
            .spacing(6)
            .into()
        } else {
            button("Analyze entire game")
                .on_press(Message::AnalyzeGame)
                .style(button::secondary)
                .into()
        };
        let review_details: Element<'_, Message> = if let Some(report) = &self.game_report {
            let entries: Vec<Element<'_, Message>> = report
                .moves
                .iter()
                .enumerate()
                .map(|(index, review)| {
                    let loss = review
                        .centipawn_loss
                        .map(|v| format!(" · {v} cp"))
                        .unwrap_or_default();
                    button(
                        text(format!(
                            "{}  ·  {}{}",
                            review.label, review.classification, loss
                        ))
                        .size(13)
                        .width(Length::Fill),
                    )
                    .on_press(Message::ReportMove(index))
                    .style(button::text)
                    .width(Length::Fill)
                    .padding([5, 6])
                    .into()
                })
                .collect();
            column![
                text("GAME REVIEW").size(11).color(muted()),
                text("Approximate engine evaluation • select a move to inspect")
                    .size(11)
                    .color(muted()),
                container(scrollable(column(entries)).height(160))
                    .padding(8)
                    .width(Length::Fill)
                    .style(card_style),
            ]
            .spacing(6)
            .into()
        } else {
            iced::widget::space().height(0).into()
        };
        let panel: Element<'_, Message> = if self.settings_open {
            column![
                text("Appearance").size(24),
                text("Make the board your own").size(13).color(muted()),
                container(
                    column![
                        text("BOARD THEME").size(11).color(muted()),
                        pick_list(
                            [BoardTheme::Forest, BoardTheme::Walnut, BoardTheme::Slate],
                            Some(self.preferences.board),
                            Message::BoardTheme
                        )
                        .width(Length::Fill),
                        text("PIECE SET").size(11).color(muted()),
                        pick_list(
                            [PieceSet::Livius, PieceSet::Meridian],
                            Some(self.preferences.pieces),
                            Message::PieceSet
                        )
                        .width(Length::Fill),
                        checkbox(self.preferences.coordinates)
                            .label("Show board coordinates")
                            .on_toggle(Message::Coordinates),
                    ]
                    .spacing(12)
                )
                .padding(18)
                .width(Length::Fill)
                .style(card_style),
                text(
                    self.settings_error
                        .as_deref()
                        .unwrap_or("Settings are stored on this device.")
                )
                .size(12)
                .color(if self.settings_error.is_some() {
                    Color::from_rgb8(235, 145, 140)
                } else {
                    muted()
                }),
                button("Back to game")
                    .on_press(Message::Appearance(false))
                    .padding([9, 12]),
            ]
            .spacing(14)
            .width(Length::Fill)
            .into()
        } else {
            column![
                text(heading).size(24),
                text(mode_description).size(13).color(muted()),
                container(
                    column![
                        text(status).size(11).color(accent()),
                        text(&self.notice).size(14)
                    ]
                    .spacing(6)
                )
                .padding(14)
                .width(Length::Fill)
                .style(card_style),
                row![
                    button("New game").on_press(Message::New).padding([9, 12]),
                    button("Undo")
                        .on_press(Message::Undo)
                        .style(button::secondary)
                        .padding([9, 12]),
                    button("Flip board")
                        .on_press(Message::Flip)
                        .style(button::secondary)
                        .padding([9, 12]),
                ]
                .spacing(6),
                text("ENTER A MOVE").size(11).color(muted()),
                row![
                    text_input("e2e4", &self.move_input)
                        .on_input(Message::MoveTextChanged)
                        .on_submit(Message::SubmitMove)
                        .width(Length::Fill),
                    button("Move")
                        .on_press(Message::SubmitMove)
                        .style(button::secondary)
                        .padding([9, 12])
                ]
                .spacing(8),
                text("GAME SETTINGS").size(11).color(muted()),
                row![
                    column![
                        text("Play as").size(12).color(muted()),
                        pick_list(
                            [Player::White, Player::Black],
                            Some(self.player),
                            Message::Player
                        )
                        .width(Length::Fill)
                    ]
                    .spacing(5)
                    .width(Length::Fill),
                    column![
                        text("Engine depth").size(12).color(muted()),
                        pick_list([4, 6, 8, 10, 12, 14, 16], Some(self.depth), Message::Depth)
                            .width(Length::Fill)
                    ]
                    .spacing(5)
                    .width(Length::Fill),
                ]
                .spacing(10),
                row![
                    text("MOVES").size(11).color(muted()).width(Length::Fill),
                    text(format!("{current_ply} / {total_moves} plies"))
                        .size(11)
                        .color(muted())
                ]
                .align_y(iced::Alignment::Center),
                row![
                    button("|<")
                        .on_press(Message::Review(0))
                        .style(button::secondary),
                    button("<")
                        .on_press(Message::Review(current_ply.saturating_sub(1)))
                        .style(button::secondary),
                    button(">")
                        .on_press(Message::Review((current_ply + 1).min(total_moves)))
                        .style(button::secondary),
                    button(">|")
                        .on_press(Message::Review(total_moves))
                        .style(button::secondary),
                ]
                .spacing(6),
                container(scrollable(move_list).height(110))
                    .padding(12)
                    .width(Length::Fill)
                    .style(card_style),
                review_control,
                review_details,
                row![
                    button("Open PGN")
                        .on_press(Message::Open)
                        .style(button::secondary)
                        .padding([9, 12]),
                    button("Save PGN")
                        .on_press(Message::Save)
                        .style(button::secondary)
                        .padding([9, 12]),
                    button("Analyze")
                        .on_press(Message::Analyze)
                        .style(button::secondary)
                        .padding([9, 12]),
                ]
                .spacing(6),
                button("Restart position")
                    .on_press(Message::Reset)
                    .style(button::text),
            ]
            .spacing(10)
            .width(Length::Fill)
            .into()
        };
        let content: Element<'_, Message> = if compact {
            scrollable(column![board_area, panel].spacing(24))
                .height(Length::Fill)
                .into()
        } else {
            row![
                board_area,
                scrollable(panel).width(340).height(Length::Fill)
            ]
            .spacing(28)
            .into()
        };
        container(column![top, content].spacing(24).padding(24))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

fn muted() -> Color {
    Color::from_rgb8(149, 160, 154)
}

fn board_color(theme: BoardTheme, light: bool) -> Color {
    match (theme, light) {
        (BoardTheme::Forest, true) => Color::from_rgb8(232, 231, 210),
        (BoardTheme::Forest, false) => Color::from_rgb8(113, 145, 119),
        (BoardTheme::Walnut, true) => Color::from_rgb8(237, 218, 187),
        (BoardTheme::Walnut, false) => Color::from_rgb8(151, 104, 75),
        (BoardTheme::Slate, true) => Color::from_rgb8(218, 226, 226),
        (BoardTheme::Slate, false) => Color::from_rgb8(103, 125, 137),
    }
}

fn accent() -> Color {
    Color::from_rgb8(160, 195, 137)
}

fn card_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgb8(33, 41, 40).into()),
        border: iced::border::rounded(8),
        ..container::Style::default()
    }
}

// Handles are reused between frames so the renderer can cache the SVG data.
fn piece_handle(set: PieceSet, color: Side, piece: Role) -> &'static svg::Handle {
    static PIECES: OnceLock<[svg::Handle; 24]> = OnceLock::new();
    let pieces = PIECES.get_or_init(|| {
        [
            include_bytes!("../assets/pieces/livius/wk.svg").as_slice(),
            include_bytes!("../assets/pieces/livius/wq.svg").as_slice(),
            include_bytes!("../assets/pieces/livius/wr.svg").as_slice(),
            include_bytes!("../assets/pieces/livius/wb.svg").as_slice(),
            include_bytes!("../assets/pieces/livius/wn.svg").as_slice(),
            include_bytes!("../assets/pieces/livius/wp.svg").as_slice(),
            include_bytes!("../assets/pieces/livius/bk.svg").as_slice(),
            include_bytes!("../assets/pieces/livius/bq.svg").as_slice(),
            include_bytes!("../assets/pieces/livius/br.svg").as_slice(),
            include_bytes!("../assets/pieces/livius/bb.svg").as_slice(),
            include_bytes!("../assets/pieces/livius/bn.svg").as_slice(),
            include_bytes!("../assets/pieces/livius/bp.svg").as_slice(),
            include_bytes!("../assets/pieces/meridian/wk.svg").as_slice(),
            include_bytes!("../assets/pieces/meridian/wq.svg").as_slice(),
            include_bytes!("../assets/pieces/meridian/wr.svg").as_slice(),
            include_bytes!("../assets/pieces/meridian/wb.svg").as_slice(),
            include_bytes!("../assets/pieces/meridian/wn.svg").as_slice(),
            include_bytes!("../assets/pieces/meridian/wp.svg").as_slice(),
            include_bytes!("../assets/pieces/meridian/bk.svg").as_slice(),
            include_bytes!("../assets/pieces/meridian/bq.svg").as_slice(),
            include_bytes!("../assets/pieces/meridian/br.svg").as_slice(),
            include_bytes!("../assets/pieces/meridian/bb.svg").as_slice(),
            include_bytes!("../assets/pieces/meridian/bn.svg").as_slice(),
            include_bytes!("../assets/pieces/meridian/bp.svg").as_slice(),
        ]
        .map(svg::Handle::from_memory)
    });
    let offset = if color == Side::White { 0 } else { 6 };
    let index = match piece {
        Role::King => 0,
        Role::Queen => 1,
        Role::Rook => 2,
        Role::Bishop => 3,
        Role::Knight => 4,
        Role::Pawn => 5,
    };
    let set_offset = if set == PieceSet::Livius { 0 } else { 12 };
    &pieces[set_offset + offset + index]
}

// Return the shaft endpoint and the two corners of a scalable arrowhead.
fn arrow_head(from: Point, to: Point, cell: f32) -> Option<(Point, Point, Point)> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f32::EPSILON {
        return None;
    }
    let (x, y) = (dx / length, dy / length);
    let tip = Point::new(to.x - x * cell * 0.28, to.y - y * cell * 0.28);
    let half = cell * 0.18;
    Some((
        tip,
        Point::new(tip.x - y * half, tip.y + x * half),
        Point::new(tip.x + y * half, tip.y - x * half),
    ))
}

#[derive(Default)]
struct BoardState {
    pressed: Option<(Square, Point)>,
    dragging: bool,
}

struct Board<'a> {
    app: &'a App,
}
impl Board<'_> {
    fn square(&self, point: Point, size: f32) -> Option<Square> {
        if size <= 0.0 || !(0.0..size).contains(&point.x) || !(0.0..size).contains(&point.y) {
            return None;
        }
        let cell = size / 8.0;
        let x = (point.x / cell) as u8;
        let y = (point.y / cell) as u8;
        let (file, rank) = if self.app.flipped {
            (7 - x, y)
        } else {
            (x, 7 - y)
        };
        Square::try_index(usize::from(rank) * 8 + usize::from(file))
    }
    fn center(&self, sq: Square, size: f32) -> Point {
        let name = sq.to_string();
        let b = name.as_bytes();
        let (f, r) = (b[0] - b'a', b[1] - b'1');
        let (x, y) = if self.app.flipped {
            (7 - f, r)
        } else {
            (f, 7 - r)
        };
        let cell = size / 8.0;
        Point::new((f32::from(x) + 0.5) * cell, (f32::from(y) + 0.5) * cell)
    }
}
impl canvas::Program<Message> for Board<'_> {
    type State = BoardState;
    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let size = bounds.width.min(bounds.height);
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let position = cursor.position_in(bounds)?;
                let sq = self.square(position, size)?;
                state.pressed = Some((sq, position));
                state.dragging = false;
                Some(canvas::Action::publish(Message::Click(sq)).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let (Some((_, origin)), Some(position)) =
                    (state.pressed, cursor.position_from(bounds.position()))
                {
                    let dx = position.x - origin.x;
                    let dy = position.y - origin.y;
                    if dx * dx + dy * dy > 25.0 {
                        state.dragging = true;
                    }
                    return Some(canvas::Action::request_redraw());
                }
                None
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let (from, _) = state.pressed.take()?;
                state.dragging = false;
                if let Some(to) = cursor
                    .position_in(bounds)
                    .and_then(|p| self.square(p, size))
                {
                    if from != to {
                        return Some(
                            canvas::Action::publish(Message::Drag(from, to)).and_capture(),
                        );
                    }
                }
                Some(canvas::Action::request_redraw())
            }
            canvas::Event::Mouse(mouse::Event::CursorLeft) => {
                state.pressed = None;
                state.dragging = false;
                Some(canvas::Action::request_redraw())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        // Separate overlay keeps hints and capture targets above SVG pieces.
        let mut overlay = Frame::new(renderer, bounds.size());
        let size = bounds.width.min(bounds.height);
        let cell = size / 8.0;
        let dragged = state.pressed.and_then(|(square, _)| {
            (state.dragging
                && self.app.selected == Some(square)
                && !self.app.busy
                && self.app.pending_promotion.is_none())
            .then(|| cursor.position_from(bounds.position()).map(|p| (square, p)))
            .flatten()
        });
        let targets = self
            .app
            .selected
            .map(|s| self.app.game.legal_targets(s))
            .unwrap_or_default();
        let last = self.app.game.last_move();
        for y in 0..8 {
            for x in 0..8 {
                let top_left = Point::new(x as f32 * cell, y as f32 * cell);
                let Some(square) = self.square(
                    Point::new(top_left.x + cell / 2.0, top_left.y + cell / 2.0),
                    size,
                ) else {
                    continue;
                };
                let is_last = matches!(last, Some((from, to)) if square == from || square == to);
                let light = (x + y) % 2 == 0;
                let in_check = self
                    .app
                    .game
                    .position
                    .king(self.app.game.position.side_to_move())
                    == square
                    && !self.app.game.position.checkers().is_empty();
                let color = if in_check {
                    Color::from_rgb8(198, 105, 91)
                } else if self.app.selected == Some(square) {
                    Color::from_rgb8(225, 198, 108)
                } else if is_last {
                    if light {
                        Color::from_rgb8(211, 214, 139)
                    } else {
                        Color::from_rgb8(163, 179, 105)
                    }
                } else {
                    board_color(self.app.preferences.board, light)
                };
                frame.fill_rectangle(top_left, Size::new(cell, cell), color);
                if targets.contains(&square) {
                    let center = Point::new(top_left.x + cell / 2.0, top_left.y + cell / 2.0);
                    if self.app.game.position.piece_on(square).is_some() {
                        overlay.stroke(
                            &Path::circle(center, cell * 0.44),
                            Stroke::default()
                                .with_width(cell * 0.08)
                                .with_color(Color::from_rgba8(24, 46, 35, 0.55)),
                        );
                    } else {
                        frame.fill(
                            &Path::circle(center, cell * 0.14),
                            Color::from_rgba8(24, 46, 35, 0.35),
                        );
                    }
                }
                if dragged.is_none_or(|(dragged_square, _)| dragged_square != square) {
                    if let (Some(color), Some(piece)) = (
                        self.app.game.position.color_on(square),
                        self.app.game.position.piece_on(square),
                    ) {
                        frame.draw_svg(
                            Rectangle {
                                x: top_left.x + cell * 0.025,
                                y: top_left.y + cell * 0.025,
                                width: cell * 0.95,
                                height: cell * 0.95,
                            },
                            piece_handle(self.app.preferences.pieces, color, piece),
                        );
                    }
                }
                let name = square.to_string();
                let label_color = if light {
                    Color::from_rgb8(99, 127, 101)
                } else {
                    Color::from_rgb8(226, 230, 208)
                };
                if self.app.preferences.coordinates && x == 0 {
                    frame.fill_text(canvas::Text {
                        content: name[1..2].into(),
                        position: Point::new(top_left.x + cell * 0.06, top_left.y + cell * 0.04),
                        size: (cell * 0.17).into(),
                        color: label_color,
                        ..canvas::Text::default()
                    });
                }
                if self.app.preferences.coordinates && y == 7 {
                    frame.fill_text(canvas::Text {
                        content: name[..1].into(),
                        position: Point::new(top_left.x + cell * 0.82, top_left.y + cell * 0.79),
                        size: (cell * 0.17).into(),
                        color: label_color,
                        ..canvas::Text::default()
                    });
                }
            }
        }
        if let Some((square, position)) = dragged {
            if let (Some(color), Some(piece)) = (
                self.app.game.position.color_on(square),
                self.app.game.position.piece_on(square),
            ) {
                frame.draw_svg(
                    Rectangle {
                        x: position.x - cell * 0.475,
                        y: position.y - cell * 0.475,
                        width: cell * 0.95,
                        height: cell * 0.95,
                    },
                    piece_handle(self.app.preferences.pieces, color, piece),
                );
            }
        }
        let hint = self.app.analysis.as_ref().and_then(|analysis| {
            Some((
                analysis.best_move.get(..2)?.parse().ok()?,
                analysis.best_move.get(2..4)?.parse().ok()?,
            ))
        });
        if let Some((from, to)) = hint {
            let a = self.center(from, size);
            let b = self.center(to, size);
            if let Some((tip, left, right)) = arrow_head(a, b, cell) {
                let orange = Color::from_rgba8(242, 153, 45, 0.85);
                overlay.stroke(
                    &Path::line(a, tip),
                    Stroke::default().with_width(cell * 0.11).with_color(orange),
                );
                overlay.fill(
                    &Path::new(|builder| {
                        builder.move_to(b);
                        builder.line_to(left);
                        builder.line_to(right);
                        builder.close();
                    }),
                    orange,
                );
            }
        }
        vec![frame.into_geometry(), overlay.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_coordinates_follow_player_orientation() {
        let mut app = App::default();
        let board = Board { app: &app };
        assert_eq!(board.square(Point::new(0.0, 0.0), 512.0), Some(Square::A8));
        assert_eq!(
            board.square(Point::new(511.0, 511.0), 512.0),
            Some(Square::H1)
        );
        assert_eq!(board.square(Point::new(512.0, 0.0), 512.0), None);
        assert_eq!(board.square(Point::new(-1.0, 0.0), 512.0), None);
        for size in [80.0, 300.0, 512.0, 900.0] {
            for square in [Square::A1, Square::E4, Square::H8] {
                assert_eq!(board.square(board.center(square, size), size), Some(square));
            }
        }
        app.flipped = true;
        let board = Board { app: &app };
        assert_eq!(board.square(Point::new(0.0, 0.0), 512.0), Some(Square::H1));
        assert_eq!(
            board.square(Point::new(511.0, 511.0), 512.0),
            Some(Square::A8)
        );
        for size in [80.0, 300.0, 512.0, 900.0] {
            for square in [Square::A1, Square::E4, Square::H8] {
                assert_eq!(board.square(board.center(square, size), size), Some(square));
            }
        }
    }

    #[test]
    fn hint_arrow_is_scalable_and_handles_zero_length() {
        let from = Point::new(0.0, 0.0);
        let to = Point::new(100.0, 0.0);
        let (tip, left, right) = arrow_head(from, to, 50.0).unwrap();
        assert!((tip.x - 86.0).abs() < 0.01);
        assert!(left.y > 0.0 && right.y < 0.0);
        assert!(arrow_head(from, from, 50.0).is_none());
    }

    #[test]
    fn flipping_preserves_game_and_player() {
        let mut app = App::default();
        app.game.play_uci("e2e4").unwrap();
        let fen = app.game.fen();
        let _ = app.update(Message::Flip);
        assert!(app.flipped);
        assert_eq!(app.player, Player::White);
        assert_eq!(app.game.fen(), fen);
        let _ = app.update(Message::Flip);
        assert!(!app.flipped);
    }

    #[test]
    fn all_piece_images_are_distinct() {
        let mut ids = Vec::new();
        for color in [Side::White, Side::Black] {
            for piece in [
                Role::King,
                Role::Queen,
                Role::Rook,
                Role::Bishop,
                Role::Knight,
                Role::Pawn,
            ] {
                ids.push(piece_handle(PieceSet::Livius, color, piece).id());
                ids.push(piece_handle(PieceSet::Meridian, color, piece).id());
            }
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 24);
    }

    #[test]
    fn promotion_waits_for_choice_and_supports_underpromotion() {
        let mut app = App::default();
        app.game = Game::from_pgn(b"[FEN \"7k/P7/5K2/8/8/8/8/8 w - - 0 1\"]\n\n*").unwrap();
        let before = app.game.fen();
        let _ = app.update(Message::Drag(Square::A7, Square::A8));
        assert_eq!(app.pending_promotion, Some((Square::A7, Square::A8)));
        assert_eq!(app.game.fen(), before);
        let _ = app.update(Message::Click(Square::F6));
        assert_eq!(app.game.fen(), before);
        let _ = app.update(Message::CancelPromotion);
        assert_eq!(app.pending_promotion, None);
        let _ = app.update(Message::Drag(Square::A7, Square::A8));
        let _ = app.update(Message::Promote(Promotion::Knight));
        assert_eq!(app.game.position.piece_on(Square::A8), Some(Role::Knight));
        assert_eq!(app.pending_promotion, None);
    }

    #[test]
    fn text_move_validates_and_plays() {
        let mut app = App::default();
        let _ = app.update(Message::MoveTextChanged("bad".into()));
        let _ = app.update(Message::SubmitMove);
        assert_eq!(app.game.fen(), Game::default().fen());
        assert!(app.notice.contains("Enter a move"));
        let _ = app.update(Message::MoveTextChanged("e2e4".into()));
        let _ = app.update(Message::SubmitMove);
        assert_eq!(app.game.moves(), ["e4"]);
        assert!(app.move_input.is_empty());
        assert!(app.busy);
    }

    #[test]
    fn canvas_drag_emits_click_and_drag_at_scaled_coordinates() {
        use iced::widget::canvas::Program;
        let app = App::default();
        let board = Board { app: &app };
        let mut state = BoardState::default();
        let bounds = Rectangle {
            x: 10.0,
            y: 20.0,
            width: 400.0,
            height: 400.0,
        };
        let from = board.center(Square::E2, 400.0);
        let to = board.center(Square::E4, 400.0);
        let cursor =
            |p: Point| mouse::Cursor::Available(Point::new(p.x + bounds.x, p.y + bounds.y));
        let press = canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let action = board
            .update(&mut state, &press, bounds, cursor(from))
            .unwrap();
        assert!(matches!(
            action.into_inner().0,
            Some(Message::Click(Square::E2))
        ));
        let motion = canvas::Event::Mouse(mouse::Event::CursorMoved { position: to });
        let _ = board.update(&mut state, &motion, bounds, cursor(to));
        assert!(state.dragging);
        let release = canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let action = board
            .update(&mut state, &release, bounds, cursor(to))
            .unwrap();
        assert!(matches!(
            action.into_inner().0,
            Some(Message::Drag(Square::E2, Square::E4))
        ));
        assert!(!state.dragging);
        assert!(state.pressed.is_none());
    }

    #[test]
    fn game_review_does_not_interrupt_a_live_engine_move() {
        let mut app = App::default();
        app.game.play_uci("e2e4").unwrap();
        app.busy = true;
        let cancel = Arc::new(AtomicBool::new(false));
        app.search_cancel = Some(Arc::clone(&cancel));
        let generation = app.generation;
        let _ = app.update(Message::AnalyzeGame);
        assert!(app.busy);
        assert_eq!(app.generation, generation);
        assert!(!cancel.load(Ordering::Relaxed));
        assert!(app.notice.contains("wait for the engine"));
    }

    #[test]
    fn game_report_review_and_cancellation_keep_position_safe() {
        let mut app = App::default();
        app.game.play_uci("e2e4").unwrap();
        app.game.play_uci("e7e5").unwrap();
        let final_fen = app.game.fen();
        app.game_report = Some(GameReport {
            moves: vec![report::MoveReview {
                label: "1. e4".into(),
                before: Analysis {
                    best_move: "d2d4".into(),
                    score: "25 cp".into(),
                    depth: 4,
                },
                classification: "Good",
                centipawn_loss: Some(12),
            }],
        });
        let _ = app.update(Message::ReportMove(0));
        assert_eq!(app.game.fen(), Game::default().fen());
        assert_eq!(app.analysis.as_ref().unwrap().best_move, "d2d4");
        assert!(app.game_report.is_some());
        let _ = app.update(Message::Review(2));
        assert_eq!(app.game.fen(), final_fen);
        let cancel = Arc::new(AtomicBool::new(false));
        app.search_cancel = Some(Arc::clone(&cancel));
        app.report_progress = Some(Arc::new(AtomicUsize::new(1)));
        app.busy = true;
        let _ = app.update(Message::CancelAnalysis);
        assert!(cancel.load(Ordering::Relaxed));
        assert!(app.report_progress.is_none());
        assert!(!app.busy);
        let _ = app.update(Message::New);
        assert!(app.game_report.is_none());
    }

    #[test]
    fn review_preserves_future_and_blocks_accidental_moves() {
        let mut app = App::default();
        app.game.play_uci("e2e4").unwrap();
        app.game.play_uci("e7e5").unwrap();
        let final_fen = app.game.fen();
        let _ = app.update(Message::Review(0));
        assert_eq!(app.game.fen(), Game::default().fen());
        let _ = app.update(Message::Drag(Square::D2, Square::D4));
        assert_eq!(app.game.fen(), Game::default().fen());
        let _ = app.update(Message::Review(1));
        assert_eq!(app.game.moves(), ["e4"]);
        let _ = app.update(Message::Review(2));
        assert_eq!(app.game.fen(), final_fen);
        assert!(app.review.is_none());
    }

    #[test]
    fn undo_takes_back_human_and_engine_moves_and_cancels_search() {
        let mut app = App::default();
        app.game.play_uci("e2e4").unwrap();
        app.game.play_uci("e7e5").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        app.search_cancel = Some(Arc::clone(&cancel));
        app.busy = true;
        let old_generation = app.generation;
        let _ = app.update(Message::Undo);
        assert_eq!(app.game.fen(), Game::default().fen());
        assert!(cancel.load(Ordering::Relaxed));
        assert!(!app.busy);
        assert_ne!(app.generation, old_generation);
        let _ = app.update(Message::Engine(old_generation, Err("stale".into())));
        assert_eq!(app.notice, "Move undone");
    }

    #[test]
    fn engine_reply_is_applied_and_error_is_shown() {
        let mut app = App::default();
        app.game.play_uci("e2e4").unwrap();
        app.busy = true;
        let _ = app.update(Message::Engine(
            app.generation,
            Ok(Analysis {
                best_move: "e7e5".into(),
                score: "0.00".into(),
                depth: 4,
            }),
        ));
        assert_eq!(app.game.moves(), ["e4", "e5"]);
        assert!(!app.busy);
        let _ = app.update(Message::Engine(
            app.generation,
            Err("engine unavailable".into()),
        ));
        assert_eq!(app.notice, "engine unavailable");
    }
}
