//! Native user interface.

pub fn run() -> iced::Result {
    iced::run((), |_| iced::widget::text("Ferrum Chess"))
}
