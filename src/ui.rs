//! Iced MVU application and interactive board.
use cozy_chess::{Color as Side, Piece as Role, Square};
use ferrum_chess::{
    engine::{self, Analysis},
    game::Game,
};
use iced::{
    futures::channel::oneshot,
    mouse,
    widget::{
        button,
        canvas::{self, Canvas, Frame, Geometry, Path, Stroke},
        column, container, pick_list, responsive, row, scrollable, svg, text,
    },
    Color, Element, Length, Point, Rectangle, Renderer, Size, Task, Theme,
};
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
};

pub fn run() -> iced::Result {
    iced::application(App::default, App::update, App::view)
        .title("Ferrum Chess")
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
        .window_size(Size::new(1200.0, 800.0))
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
            flipped: false,
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
    Flip,
    New,
    Reset,
    Undo,
    Analyze,
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
                self.flipped = player == Player::Black;
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
            Message::Flip => self.flipped = !self.flipped,
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
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let board = Canvas::new(Board { app: self })
            .width(board_size)
            .height(board_size);
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

        let moves = self.game.moves();
        let move_text = if moves.is_empty() {
            "Your moves will appear here.".to_owned()
        } else {
            moves
                .chunks(2)
                .enumerate()
                .map(|(i, pair)| {
                    format!(
                        "{:>2}.  {:<8} {}",
                        i + 1,
                        pair[0],
                        pair.get(1).map_or("", String::as_str)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
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
        } else if self.game.is_game_over() {
            "GAME OVER"
        } else {
            "READY"
        };
        let panel = column![
            text(heading).size(24),
            text(mode_description).size(13).color(muted()),
            container(
                column![
                    text(status).size(11).color(accent()),
                    text(&self.notice).size(14),
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
                column![
                    text("Pawn promotion").size(12).color(muted()),
                    pick_list(
                        [
                            Promotion::Queen,
                            Promotion::Rook,
                            Promotion::Bishop,
                            Promotion::Knight
                        ],
                        Some(self.promotion),
                        Message::Promotion
                    )
                    .width(Length::Fill)
                ]
                .spacing(5)
                .width(Length::Fill),
                button("Restart")
                    .on_press(Message::Reset)
                    .style(button::secondary)
                    .padding([9, 12]),
            ]
            .spacing(10)
            .align_y(iced::Alignment::End),
            row![
                text("MOVES").size(11).color(muted()).width(Length::Fill),
                text(format!("{} plies", moves.len()))
                    .size(11)
                    .color(muted())
            ]
            .align_y(iced::Alignment::Center),
            container(scrollable(text(move_text).size(15)).height(110))
                .padding(12)
                .width(Length::Fill)
                .style(card_style),
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
        ]
        .spacing(10)
        .width(Length::Fill);
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
fn piece_handle(color: Side, piece: Role) -> &'static svg::Handle {
    static PIECES: OnceLock<[svg::Handle; 12]> = OnceLock::new();
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
    &pieces[offset + index]
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
    type State = Option<Square>;
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
                let sq = self.square(cursor.position_in(bounds)?, size)?;
                *state = Some(sq);
                Some(canvas::Action::publish(Message::Click(sq)).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let from = state.take()?;
                let to = self.square(cursor.position_in(bounds)?, size)?;
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
        let size = bounds.width.min(bounds.height);
        let cell = size / 8.0;
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
                let color = if self.app.selected == Some(square) {
                    Color::from_rgb8(225, 198, 108)
                } else if is_last {
                    if light {
                        Color::from_rgb8(211, 214, 139)
                    } else {
                        Color::from_rgb8(163, 179, 105)
                    }
                } else if light {
                    Color::from_rgb8(232, 231, 210)
                } else {
                    Color::from_rgb8(113, 145, 119)
                };
                frame.fill_rectangle(top_left, Size::new(cell, cell), color);
                if targets.contains(&square) {
                    frame.fill(
                        &Path::circle(
                            Point::new(top_left.x + cell / 2.0, top_left.y + cell / 2.0),
                            cell * 0.14,
                        ),
                        Color::from_rgba8(24, 46, 35, 0.35),
                    );
                }
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
                        piece_handle(color, piece),
                    );
                }
                let name = square.to_string();
                let label_color = if light {
                    Color::from_rgb8(99, 127, 101)
                } else {
                    Color::from_rgb8(226, 230, 208)
                };
                if x == 0 {
                    frame.fill_text(canvas::Text {
                        content: name[1..2].into(),
                        position: Point::new(top_left.x + cell * 0.06, top_left.y + cell * 0.04),
                        size: (cell * 0.17).into(),
                        color: label_color,
                        ..canvas::Text::default()
                    });
                }
                if y == 7 {
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
        let hint = self.app.analysis.as_ref().and_then(|analysis| {
            Some((
                analysis.best_move.get(..2)?.parse().ok()?,
                analysis.best_move.get(2..4)?.parse().ok()?,
            ))
        });
        if let Some((from, to)) = hint {
            let a = self.center(from, size);
            let b = self.center(to, size);
            frame.stroke(
                &Path::line(a, b),
                Stroke::default()
                    .with_width(cell * 0.11)
                    .with_color(Color::from_rgba8(242, 153, 45, 0.8)),
            );
            frame.fill(
                &Path::circle(b, cell * 0.13),
                Color::from_rgb8(242, 153, 45),
            );
        }
        vec![frame.into_geometry()]
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
                ids.push(piece_handle(color, piece).id());
            }
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 12);
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
