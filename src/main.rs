pub mod content;

use clap::Parser;
use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::{Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Stylize},
    widgets::{Block, Clear, Paragraph, Widget, Wrap},
};
use std::{borrow::Cow, path::PathBuf};

use crate::content::{CellLocation, CsvTable};

const POPUP_WIDTH_PERCENT: u16 = 30;
const POPUP_HEIGHT_PERCENT: u16 = 30;

fn main() -> color_eyre::Result<()> {
    let args = Args::parse();
    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = App::new().run(terminal, args);
    ratatui::restore();
    result
}

/// The main application which holds the state and logic of the application.
#[derive(Debug, Default)]
struct App {
    /// Is the application running?
    running: bool,
    input_state: InputState,
    csv_table: Option<CsvTable>,
    pop_up: Option<Popup>,
}
#[derive(Clone, Copy, Debug, Default)]
enum InputState {
    #[default]
    Normal,
    Console,
}

impl App {
    /// Construct a new instance of [`App`].
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_load_table(&mut self, file: PathBuf) -> color_eyre::Result<()> {
        self.csv_table = Some(CsvTable::load_from_file(file)?);
        Ok(())
    }

    /// Run the application's main loop.
    fn run(mut self, mut terminal: DefaultTerminal, args: Args) -> Result<()> {
        self.running = true;
        if let Some(file) = args.file {
            let res = self.try_load_table(file);
            if let Err(err) = res {
                self.pop_up = Some(Popup::error(format!("{err}")));
            }
        }
        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            self.handle_crossterm_events()?;
        }
        Ok(())
    }

    /// Renders the user interface.
    ///
    /// This is where you add new widgets. See the following resources for more information:
    ///
    /// - <https://docs.rs/ratatui/latest/ratatui/widgets/index.html>
    /// - <https://github.com/ratatui/ratatui/tree/main/ratatui-widgets/examples>
    fn render(&mut self, frame: &mut Frame) {
        let [main_area, console_bar] =
            Layout::vertical(vec![Constraint::Percentage(98), Constraint::Max(2)])
                .areas(frame.area());
        frame.render_widget(Block::new(), main_area);

        frame.render_widget(
            Grid {
                cols: 10,
                rows: 10,
                cell_height: 3,
                cell_width: 15,
                top_left_cell_location: CellLocation { row: 0, col: 0 },
                csv_table: self.csv_table.as_ref(),
            },
            main_area,
        );
        if let Some(popup) = &self.pop_up {
            frame.render_widget(popup, frame.area());
        }
    }

    /// Reads the crossterm events and updates the state of [`App`].
    ///
    /// If your application needs to perform work in between handling events, you can use the
    /// [`event::poll`] function to check if there are any events available with a timeout.
    fn handle_crossterm_events(&mut self) -> Result<()> {
        match event::read()? {
            // it's important to check KeyEventKind::Press to avoid handling key release events
            Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key_event(key),
            Event::Mouse(_) => {}
            Event::Resize(_, _) => {}
            _ => {}
        }
        Ok(())
    }

    /// Handles the key events and updates the state of [`App`].
    fn on_key_event(&mut self, key: KeyEvent) {
        #[allow(clippy::single_match)]
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q')) => self.quit(),
            (_, KeyCode::Char(':')) => self.input_state = InputState::Console,
            (_, KeyCode::Esc) => {
                if self.pop_up.is_some() {
                    self.pop_up = None;
                } else {
                    self.input_state = InputState::Normal;
                }
            }
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
                if let Some(csv_table) = &mut self.csv_table {
                    csv_table
                        .normalize_and_save()
                        .inspect_err(|err| self.pop_up = Some(Popup::error(format!("{err}"))))
                        .ok();
                }
            }
            _ => {}
        }
    }

    /// Set running to false to quit the application.
    fn quit(&mut self) {
        self.running = false;
    }
}

struct Grid<'a> {
    cols: usize,
    rows: usize,
    cell_height: u16,
    cell_width: u16,
    top_left_cell_location: CellLocation,
    csv_table: Option<&'a CsvTable>,
}

/// https://ratatui.rs/recipes/layout/grid/
impl<'a> Widget for Grid<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let col_constraints = (0..self.cols).map(|_| Constraint::Length(self.cell_width));
        let row_constraints = (0..self.rows).map(|_| Constraint::Length(self.cell_height));
        let horizontal = Layout::horizontal(col_constraints).spacing(0);
        let vertical = Layout::vertical(row_constraints).spacing(0);

        let rows = vertical.split(area);
        let cells = rows.iter().flat_map(|&row| horizontal.split(row).to_vec());

        // let cells = area
        //     .layout_vec(&vertical)
        //     .iter()
        //     .flat_map(|row| row.layout_vec(&horizontal));

        for (i, cell) in cells.enumerate() {
            let text = if let Some(csv_table) = self.csv_table {
                let row = i / self.cols;
                let col = i % self.cols;
                csv_table
                    .get(self.top_left_cell_location + CellLocation { row, col })
                    .unwrap_or_default()
            } else {
                Default::default()
            };
            Paragraph::new(text)
                .block(Block::bordered())
                .render(cell, buf);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Popup {
    severity: Severity,
    message: Cow<'static, str>,
}

impl Popup {
    #[allow(unused)]
    pub(crate) fn new(message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            message: message.into(),
            ..Default::default()
        }
    }

    #[allow(unused)]
    pub(crate) fn error(message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            message: message.into(),
            severity: Severity::Error,
        }
    }

    #[allow(unused)]
    pub(crate) fn warning(message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            message: message.into(),
            severity: Severity::Warning,
        }
    }

    #[allow(unused)]
    pub(crate) fn success(message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            message: message.into(),
            severity: Severity::Success,
        }
    }
}

impl Widget for &Popup {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let vertical =
            Layout::vertical([Constraint::Percentage(POPUP_HEIGHT_PERCENT)]).flex(Flex::Center);
        let horizontal =
            Layout::horizontal([Constraint::Percentage(POPUP_WIDTH_PERCENT)]).flex(Flex::Center);
        let [area] = vertical.areas(area);
        let [area] = horizontal.areas(area);
        let bg = match self.severity {
            Severity::Neutral => Color::DarkGray,
            Severity::Success => Color::Green,
            Severity::Warning => Color::Yellow,
            Severity::Error => Color::Red,
        };
        let block = Block::bordered().bg(bg);
        let paragraph = Paragraph::new(self.message.clone())
            .fg(Color::White)
            .block(block);
        Clear.render(area, buf);
        paragraph.render(area, buf);
    }
}

#[derive(Clone, Copy, Debug, Default)]
enum Severity {
    #[default]
    Neutral,
    Success,
    Warning,
    Error,
}

#[derive(Parser, Debug)]
struct Args {
    #[arg()]
    file: Option<PathBuf>,
}
