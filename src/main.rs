pub mod content;

use clap::Parser;
use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Stylize},
    widgets::{Block, Paragraph, Widget},
};
use std::{borrow::Cow, path::PathBuf};

use crate::content::{CellLocation, CellLocationDelta, CsvTable};

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
    console_message: Option<ConsoleMessage>,
    selection: Selection,
}
#[derive(Debug, Clone, Default)]
struct Selection {
    selected: Vec<CellLocation>,
    primary: CellLocation,
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
                self.console_message = Some(ConsoleMessage::error(format!("{err}")));
            }
        }
        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            if let Err(err) = self.handle_crossterm_events() {
                self.console_message = Some(ConsoleMessage::error(format!("{err}")));
            };
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
                selection: &self.selection,
            },
            main_area,
        );

        if let InputState::Console(console) = &self.input_state {
            frame.render_widget(console, console_bar);
        } else if let Some(console_message) = &self.console_message {
            frame.render_widget(console_message, console_bar);
        }
    }

    /// Reads the crossterm events and updates the state of [`App`].
    ///
    /// If your application needs to perform work in between handling events, you can use the
    /// [`event::poll`] function to check if there are any events available with a timeout.
    fn handle_crossterm_events(&mut self) -> Result<()> {
        match event::read()? {
            // it's important to check KeyEventKind::Press to avoid handling key release events
            Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key_event(key)?,
            Event::Mouse(_) => {}
            Event::Resize(_, _) => {}
            _ => {}
        }
        Ok(())
    }

    /// Handles the key events and updates the state of [`App`].
    fn on_key_event(&mut self, key: KeyEvent) -> Result<()> {
        self.console_message = None;
        if let (_, KeyCode::Esc) = (key.modifiers, key.code) {
            if self.console_message.is_some() {
                self.console_message = None;
            } else {
                self.input_state = InputState::Normal;
            }
        }
        match &mut self.input_state {
            InputState::Normal => match (key.modifiers, key.code) {
                (_, KeyCode::Char('h')) => self.move_selection(MoveDirection::Left),
                (_, KeyCode::Char('j')) => self.move_selection(MoveDirection::Down),
                (_, KeyCode::Char('k')) => self.move_selection(MoveDirection::Up),
                (_, KeyCode::Char('l')) => self.move_selection(MoveDirection::Right),
                (_, KeyCode::Char('i')) => {
                    let content = if let Some(table) = &self.csv_table {
                        table.get(self.selection.primary).unwrap_or_default()
                    } else {
                        Default::default()
                    };
                    self.input_state = InputState::Console(Console {
                        mode: ConsoleBarMode::CellInput,
                        content: content.to_owned(),
                    })
                }
                (_, KeyCode::Char('c')) => {
                    self.input_state = InputState::Console(Console {
                        mode: ConsoleBarMode::CellInput,
                        content: Default::default(),
                    })
                }
                (_, KeyCode::Char(':')) => {
                    self.input_state = InputState::Console(Console {
                        mode: ConsoleBarMode::Console,
                        content: String::default(),
                    })
                }
                _ => {}
            },
            InputState::Console(Console { content, mode }) => match (key.modifiers, key.code) {
                (_, KeyCode::Enter) => {
                    let content = content.clone();
                    match mode {
                        ConsoleBarMode::Console => {
                            self.try_execute_command(&content)?;
                        }
                        ConsoleBarMode::CellInput => {
                            if let Some(table) = &mut self.csv_table {
                                table.set(self.selection.primary, content);
                            }
                        }
                    }

                    self.input_state = InputState::Normal;
                }
                (m, KeyCode::Char(c)) => {
                    let c = if m == KeyModifiers::SHIFT {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    };
                    content.push(c);
                }
                (_, KeyCode::Backspace) => {
                    content.pop();
                }
                _ => {}
            },
        }
        Ok(())
    }

    fn move_selection(&mut self, direction: MoveDirection) {
        let delta = match direction {
            MoveDirection::Left => CellLocationDelta { x: -1, y: 0 },
            MoveDirection::Down => CellLocationDelta { x: 0, y: 1 },
            MoveDirection::Up => CellLocationDelta { x: 0, y: -1 },
            MoveDirection::Right => CellLocationDelta { x: 1, y: 0 },
        };
        self.selection.primary = self.selection.primary + delta;
    }

    fn try_execute_command(&mut self, command: &str) -> Result<()> {
        match command.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["w" | "write", ..] => {
                if let Some(csv_table) = &mut self.csv_table {
                    csv_table.normalize_and_save()?;
                };
            }
            ["q!" | "quit!", ..] => {
                self.quit();
            }
            ["wq" | "x" | "write-quit" | "wq!" | "x!" | "write-quit!", ..] => {
                if let Some(csv_table) = &mut self.csv_table {
                    csv_table.normalize_and_save()?;
                };
                self.quit();
            }
            ["q" | "quit", ..] => {
                self.console_message = Some(ConsoleMessage::error(
                    "`quit` is not implemented - Use `quit!`",
                ))
            }
            ["o" | "open", file, ..] => {
                if let Err(err) = self.try_load_table(PathBuf::from(file)) {
                    self.console_message = Some(ConsoleMessage::error(format!("{err}")));
                }
            }
            ["bc!" | "buffer-close!", ..] => {
                self.csv_table = None;
            }
            [c, ..] => {
                self.console_message = Some(ConsoleMessage::error(format!("Unknown command: {c}")));
            }
            _ => {}
        }
        Ok(())
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
    selection: &'a Selection,
}

/// https://ratatui.rs/recipes/layout/grid/
impl<'a> Widget for Grid<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let Grid {
            cols,
            rows,
            cell_height,
            cell_width,
            top_left_cell_location,
            csv_table,
            selection,
        } = self;
        let Selection { selected, primary } = selection;
        let col_constraints = (0..cols).map(|_| Constraint::Length(cell_width));
        let row_constraints = (0..rows).map(|_| Constraint::Length(cell_height));
        let horizontal = Layout::horizontal(col_constraints).spacing(0);
        let vertical = Layout::vertical(row_constraints).spacing(0);

        let rows = vertical.split(area);
        let cells = rows.iter().flat_map(|&row| horizontal.split(row).to_vec());

        // let cells = area
        //     .layout_vec(&vertical)
        //     .iter()
        //     .flat_map(|row| row.layout_vec(&horizontal));

        for (i, cell) in cells.enumerate() {
            let row = i / cols;
            let col = i % cols;
            let cell_location = top_left_cell_location + CellLocation { row, col };
            let text = if let Some(csv_table) = csv_table {
                csv_table.get(cell_location).unwrap_or_default()
            } else {
                Default::default()
            };
            let fg = if *primary == cell_location {
                Color::LightBlue
            } else if selected.contains(&cell_location) {
                Color::Blue
            } else {
                Color::Reset
            };
            Paragraph::new(text)
                .fg(fg)
                .block(Block::bordered())
                .render(cell, buf);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ConsoleMessage {
    severity: Severity,
    message: Cow<'static, str>,
}

impl ConsoleMessage {
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

impl Widget for &ConsoleMessage {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let ConsoleMessage { severity, message } = self;
        let (prefix, color) = match *severity {
            Severity::Error => ("! ", Color::Red),
            _ => ("", Color::Reset),
        };
        let paragraph = Paragraph::new(format!("{prefix}{message}")).fg(color);

        paragraph.render(area, buf);
    }
}

#[derive(Clone, Debug)]
struct Console {
    mode: ConsoleBarMode,
    content: String,
}

impl Widget for &Console {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let Console { mode, content } = self;
        let prefix = match mode {
            ConsoleBarMode::Console => ":",
            ConsoleBarMode::CellInput => ">",
        };
        let paragraph = Paragraph::new(format!("{prefix}{content}"));
        paragraph.render(area, buf);
    }
}

#[derive(Clone, Debug, Default)]
enum InputState {
    #[default]
    Normal,
    Console(Console),
}

#[allow(unused)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConsoleBarMode {
    Console,
    CellInput,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MoveDirection {
    Left,
    Down,
    Up,
    Right,
}
