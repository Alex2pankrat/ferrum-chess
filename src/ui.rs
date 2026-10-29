//! Iced MVU application and interactive board.
use cozy_chess::{Color as Side, Piece as Role, Square};
use ferrum_chess::{
    engine::{self, Analysis},
    game::Game,
};
use iced::{
    alignment,
    futures::channel::oneshot,
    mouse,
    widget::{
        button,
        canvas::{self, Canvas, Frame, Geometry, Path, Stroke},
        column, container, pick_list, row, scrollable, text,
    },
    Color, Element, Length, Point, Rectangle, Renderer, Size, Task, Theme,
};
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub fn run() -> iced::Result {
    iced::application(App::default, App::update, App::view)
        .title("Ferrum Chess")
        .font(include_bytes!("../assets/NotoSansSymbols2-Regular.ttf").as_slice())
        .theme(Theme::Dark)
        .window_size(Size::new(1024.0, 720.0))
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
    promotion: Promotion,
    depth: u32,
    selected: Option<Square>,
    analysis: Option<Analysis>,
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
            promotion: Promotion::Queen,
            depth: 10,
            selected: None,
            analysis: None,
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
    Promotion(Promotion),
    New,
    Reset,
    Undo,
    Analyze,
    Engine(u64, Result<Analysis, String>),
    Open,
    Save,
    Opened(u64, Result<Option<Game>, String>),
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
        self.analysis = None;
        self.selected = None;
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
            || (self.mode == Mode::Play && self.game.position.side_to_move() != self.player.side())
        {
            return Task::none();
        }
        match self.game.play(from, to, self.promotion.role()) {
            Ok(()) => {
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
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Click(square) => {
                if self.busy
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
                self.invalidate();
                return self.maybe_search();
            }
            Message::Player(player) => {
                self.player = player;
                self.game = Game::default();
                self.invalidate();
                return self.maybe_search();
            }
            Message::Depth(depth) => {
                self.depth = depth;
                self.invalidate();
                return self.maybe_search();
            }
            Message::Promotion(p) => self.promotion = p,
            Message::New => {
                self.game = Game::default();
                self.invalidate();
                return self.maybe_search();
            }
            Message::Reset => {
                self.game.reset();
                self.invalidate();
                return self.maybe_search();
            }
            Message::Undo => {
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
                        {
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
                        Game::from_pgn(&data).map(Some)
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
                        self.game = game;
                        self.invalidate();
                        self.notice = "PGN loaded".into();
                        return self.maybe_search();
                    }
                    Ok(None) => {}
                    Err(e) => self.notice = format!("Cannot open PGN: {e}"),
                }
            }
            Message::Save => {
                let pgn = self.game.to_pgn();
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
        let board = Canvas::new(Board { app: self }).width(512).height(512);
        let controls = column![
            text("FERRUM CHESS").size(26),
            text("OFFLINE CHESS STUDIO").size(12),
            row![
                button("New game").on_press(Message::New),
                button("Restart").on_press(Message::Reset),
                button("Undo").on_press(Message::Undo)
            ]
            .spacing(8),
            text("Mode"),
            pick_list([Mode::Play, Mode::Analysis], Some(self.mode), Message::Mode),
            text("Play as (starts a new game)"),
            pick_list(
                [Player::White, Player::Black],
                Some(self.player),
                Message::Player
            ),
            text("Search depth"),
            pick_list([4, 6, 8, 10, 12, 14, 16], Some(self.depth), Message::Depth),
            text("Promote pawn to"),
            pick_list(
                [
                    Promotion::Queen,
                    Promotion::Rook,
                    Promotion::Bishop,
                    Promotion::Knight
                ],
                Some(self.promotion),
                Message::Promotion
            ),
            row![
                button("Open PGN").on_press(Message::Open),
                button("Save PGN").on_press(Message::Save),
                button("Analyze").on_press(Message::Analyze)
            ]
            .spacing(8),
            text(&self.notice).size(14),
            text("MOVES").size(12),
            scrollable(text(self.game.moves().join("  ")).size(16)).height(Length::Fill),
        ]
        .spacing(9)
        .width(Length::Fill);
        container(row![board, controls].spacing(24).padding(24)).into()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

struct Board<'a> {
    app: &'a App,
}
impl Board<'_> {
    fn square(&self, point: Point) -> Option<Square> {
        if !(0.0..512.0).contains(&point.x) || !(0.0..512.0).contains(&point.y) {
            return None;
        }
        let x = (point.x / 64.0) as u8;
        let y = (point.y / 64.0) as u8;
        let (file, rank) = if self.app.player == Player::White {
            (x, 7 - y)
        } else {
            (7 - x, y)
        };
        Square::try_index(usize::from(rank) * 8 + usize::from(file))
    }
    fn center(&self, sq: Square) -> Point {
        let name = sq.to_string();
        let b = name.as_bytes();
        let (f, r) = (b[0] - b'a', b[1] - b'1');
        let (x, y) = if self.app.player == Player::White {
            (f, 7 - r)
        } else {
            (7 - f, r)
        };
        Point::new(f32::from(x) * 64.0 + 32.0, f32::from(y) * 64.0 + 32.0)
    }
}
impl canvas::Program<Message> for Board<'_> {
    type State = Option<Square>;
    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let sq = self.square(cursor.position_in(bounds)?)?;
                *state = Some(sq);
                Some(canvas::Action::publish(Message::Click(sq)).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let from = state.take()?;
                let to = self.square(cursor.position_in(bounds)?)?;
                if from != to {
                    Some(canvas::Action::publish(Message::Drag(from, to)).and_capture())
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let targets = self
            .app
            .selected
            .map(|s| self.app.game.legal_targets(s))
            .unwrap_or_default();
        let last = self.app.game.last_move();
        for y in 0..8 {
            for x in 0..8 {
                let top_left = Point::new(x as f32 * 64.0, y as f32 * 64.0);
                let Some(square) = self.square(Point::new(top_left.x + 32.0, top_left.y + 32.0))
                else {
                    continue;
                };
                let is_last = matches!(last, Some((from, to)) if square == from || square == to);
                let color = if self.app.selected == Some(square) {
                    Color::from_rgb8(221, 195, 102)
                } else if is_last {
                    Color::from_rgb8(175, 184, 95)
                } else if (x + y) % 2 == 0 {
                    Color::from_rgb8(235, 230, 211)
                } else {
                    Color::from_rgb8(111, 139, 111)
                };
                frame.fill_rectangle(top_left, Size::new(64.0, 64.0), color);
                if targets.contains(&square) {
                    frame.fill(
                        &Path::circle(Point::new(top_left.x + 32.0, top_left.y + 32.0), 9.0),
                        Color::from_rgba8(24, 46, 35, 0.45),
                    );
                }
                if let (Some(color), Some(piece)) = (
                    self.app.game.position.color_on(square),
                    self.app.game.position.piece_on(square),
                ) {
                    let glyph = match (color, piece) {
                        (Side::White, Role::King) => "♔",
                        (Side::White, Role::Queen) => "♕",
                        (Side::White, Role::Rook) => "♖",
                        (Side::White, Role::Bishop) => "♗",
                        (Side::White, Role::Knight) => "♘",
                        (Side::White, Role::Pawn) => "♙",
                        (Side::Black, Role::King) => "♚",
                        (Side::Black, Role::Queen) => "♛",
                        (Side::Black, Role::Rook) => "♜",
                        (Side::Black, Role::Bishop) => "♝",
                        (Side::Black, Role::Knight) => "♞",
                        (Side::Black, Role::Pawn) => "♟",
                    };
                    frame.fill_text(canvas::Text {
                        content: glyph.into(),
                        font: iced::Font::with_name("Noto Sans Symbols 2"),
                        position: Point::new(top_left.x + 32.0, top_left.y + 32.0),
                        size: 50.0.into(),
                        color: Color::from_rgb8(25, 32, 29),
                        align_x: alignment::Horizontal::Center.into(),
                        align_y: alignment::Vertical::Center,
                        ..canvas::Text::default()
                    });
                }
            }
        }
        let hint = self.app.analysis.as_ref().and_then(|analysis| {
            Some((
                analysis.best_move.get(..2)?.parse().ok()?,
                analysis.best_move.get(2..4)?.parse().ok()?,
            ))
        });
        if let Some((from, to)) = hint {
            let a = self.center(from);
            let b = self.center(to);
            frame.stroke(
                &Path::line(a, b),
                Stroke::default()
                    .with_width(7.0)
                    .with_color(Color::from_rgba8(242, 153, 45, 0.8)),
            );
            frame.fill(&Path::circle(b, 8.0), Color::from_rgb8(242, 153, 45));
        }
        vec![frame.into_geometry()]
    }
}
