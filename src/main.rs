pub mod content;

use clap::Parser;
use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    widgets::{Block, Paragraph, Widget},
};
use std::{borrow::Cow, path::PathBuf};

use crate::content::{CellLocation, CellLocationDelta, CsvTable};

fn main() -> color_eyre::Result<()> {
    let args = Args::parse();
    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = App::new(terminal).run(args);
    ratatui::restore();
    result
}

/// The main application which holds the state and logic of the application.
#[derive(Debug)]
struct App {
    terminal: DefaultTerminal,
    state: AppState,
}

#[derive(Debug, Default)]
struct AppState {
    /// Is the application running?
    running: bool,
    input: InputState,
    console_message: Option<ConsoleMessage>,
    table: Option<CsvTableWrapper>,
    yank: Option<Yank>,
}

impl App {
    /// Construct a new instance of [`App`].
    pub fn new(terminal: DefaultTerminal) -> Self {
        Self {
            terminal,
            state: Default::default(),
        }
    }

    /// Run the application's main loop.
    fn run(mut self, args: Args) -> Result<()> {
        self.state.running = true;
        if let Some(file) = args.file {
            let res = self.try_load_table(file);
            if let Err(err) = res {
                self.state.console_message = Some(ConsoleMessage::error(format!("{err}")));
            }
        }
        while self.state.running {
            self.terminal.draw(|frame| self.state.render(frame))?;
            if let Err(err) = self.handle_crossterm_events() {
                self.state.console_message = Some(ConsoleMessage::error(format!("{err}")));
            };
        }
        Ok(())
    }

    /// Reads the crossterm events and updates the state of [`App`].
    ///
    /// If your application needs to perform work in between handling events, you can use the
    /// [`event::poll`] function to check if there are any events available with a timeout.
    fn handle_crossterm_events(&mut self) -> Result<()> {
        match event::read()? {
            // it's important to check KeyEventKind::Press to avoid handling key release events
            Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key_event(key)?,
            _ => {}
        }
        Ok(())
    }

    /// Handles the key events and updates the state of [`App`].
    fn on_key_event(&mut self, key: KeyEvent) -> Result<()> {
        self.state.console_message = None;
        if let (_, KeyCode::Esc) = (key.modifiers, key.code) {
            if self.state.console_message.is_some() {
                self.state.console_message = None;
            } else {
                self.state.input = InputState::Normal;
            }
            return Ok(());
        }
        match &self.state.input {
            InputState::Normal => match (key.modifiers, key.code) {
                (_, KeyCode::Char(':')) => {
                    self.state.input = InputState::Console(Console {
                        mode: ConsoleBarMode::Console,
                        content: String::default(),
                    })
                }
                _ if self.state.table.is_some() => self.handle_table_key_input(key)?,
                _ => {}
            },
            InputState::Console(_) => self.handle_console_input(key)?,
        }
        Ok(())
    }

    fn handle_table_key_input(&mut self, key: KeyEvent) -> Result<()> {
        let InputState::Normal = &mut self.state.input else {
            unreachable!();
        };
        let table = self.state.table.as_mut().unwrap();
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('H')) => {
                table.selection.selected = Vec::new();
                table.move_selection(MoveDirection::Left, table.visible_cols / 2);
            }
            (KeyModifiers::CONTROL, KeyCode::Char('d')) | (_, KeyCode::Char('J')) => {
                table.selection.selected = Vec::new();
                table.move_selection(MoveDirection::Down, table.visible_rows / 2);
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u')) | (_, KeyCode::Char('K')) => {
                table.selection.selected = Vec::new();
                table.move_selection(MoveDirection::Up, table.visible_rows / 2);
            }
            (_, KeyCode::Char('L')) => {
                table.selection.selected = Vec::new();
                table.move_selection(MoveDirection::Right, table.visible_cols / 2);
            }
            (_, KeyCode::Char('h')) => table.move_selection(MoveDirection::Left, 1),
            (_, KeyCode::Char('j')) => table.move_selection(MoveDirection::Down, 1),
            (_, KeyCode::Char('k')) => table.move_selection(MoveDirection::Up, 1),
            (_, KeyCode::Char('l')) => table.move_selection(MoveDirection::Right, 1),
            (_, KeyCode::Char('i')) => {
                let content = table
                    .csv_table
                    .get(table.selection.primary)
                    .unwrap_or_default();
                self.state.input = InputState::Console(Console {
                    mode: ConsoleBarMode::CellInput,
                    content: content.to_owned(),
                });
            }
            (_, KeyCode::Char('c')) => {
                self.state.input = InputState::Console(Console {
                    mode: ConsoleBarMode::CellInput,
                    content: Default::default(),
                })
            }
            (_, KeyCode::Char('y')) => {
                // TODO: implement for rectangle selections
                let content = table
                    .csv_table
                    .get(table.selection.primary)
                    .map(ToOwned::to_owned);
                let content = vec![vec![content]];
                table.selection_yanked = Some(table.selection.clone());
                self.state.yank = Some(Yank::new(content))
            }
            (_, KeyCode::Char('d')) => {
                // TODO: implement for rectangle selections
                let content = table
                    .csv_table
                    .get(table.selection.primary)
                    .map(ToOwned::to_owned);
                let content = vec![vec![content]];
                table.csv_table.set(table.selection.primary, None);
                self.state.yank = Some(Yank::new(content))
            }
            (_, KeyCode::Char('p')) => {
                // TODO: implement for rectangle selections
                if let Some(Yank { content, .. }) = &mut self.state.yank {
                    table
                        .csv_table
                        .set(table.selection.primary, content[0][0].take());
                    table.selection_yanked = None;
                }
            }
            (_, KeyCode::Char(':')) => {
                self.state.input = InputState::Console(Console {
                    mode: ConsoleBarMode::Console,
                    content: String::default(),
                })
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_console_input(&mut self, key: KeyEvent) -> Result<()> {
        let InputState::Console(Console { mode, content }) = &mut self.state.input else {
            unreachable!();
        };
        match (key.modifiers, key.code) {
            (_, KeyCode::Enter) => {
                let content = content.clone();
                match mode {
                    ConsoleBarMode::Console => {
                        self.try_execute_command(&content)?;
                    }
                    ConsoleBarMode::CellInput => {
                        if let Some(table) = &mut self.state.table {
                            table.csv_table.set(table.selection.primary, Some(content));
                        }
                    }
                }

                self.state.input = InputState::Normal;
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
        }
        Ok(())
    }

    fn try_execute_command(&mut self, command: &str) -> Result<()> {
        match command.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["w" | "write", ..] => {
                if let Some(table) = &mut self.state.table {
                    table.csv_table.normalize_and_save()?;
                };
            }
            ["q!" | "quit!", ..] => {
                self.quit();
            }
            ["wq" | "x" | "write-quit" | "wq!" | "x!" | "write-quit!", ..] => {
                if let Some(table) = &mut self.state.table {
                    table.csv_table.normalize_and_save()?;
                };
                self.quit();
            }
            ["q" | "quit", ..] => {
                if self.state.table.is_none() {
                    self.quit();
                }
                self.state.console_message = Some(ConsoleMessage::error(
                    "`quit` is not implemented - Use `quit!`",
                ))
            }
            ["o" | "open", file, ..] => {
                if let Err(err) = self.try_load_table(PathBuf::from(file)) {
                    self.state.console_message = Some(ConsoleMessage::error(format!("{err}")));
                }
            }
            ["n" | "new", ..] => {
                self.state.table = Some(CsvTableWrapper::default());
            }
            ["bc!" | "buffer-close!", ..] => {
                self.state.table = None;
            }
            [c, ..] => {
                self.state.console_message =
                    Some(ConsoleMessage::error(format!("Unknown command: {c}")));
            }
            _ => {}
        }
        Ok(())
    }

    pub fn try_load_table(&mut self, file: PathBuf) -> color_eyre::Result<()> {
        let csv_table = CsvTable::load_from_file(file)?;
        self.state.table = Some(CsvTableWrapper {
            csv_table,
            ..Default::default()
        });
        Ok(())
    }

    /// Set running to false to quit the application.
    fn quit(&mut self) {
        self.state.running = false;
    }
}

impl AppState {
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
        if let Some(table) = &mut self.table {
            table.recalculate_dimensions(main_area.width, main_area.height);
            frame.render_widget(&*table, main_area);
        }

        if let InputState::Console(console) = &self.input {
            frame.render_widget(console, console_bar);
        } else if let Some(console_message) = &self.console_message {
            frame.render_widget(console_message, console_bar);
        }
    }
}

#[derive(Debug, Clone)]
struct CsvTableWidgetStyle {
    normal_00: Style,
    normal_01: Style,
    normal_10: Style,
    normal_11: Style,
    primary_selection: Style,
    secondary_selection: Style,
    yanked: Style,
}

impl Default for CsvTableWidgetStyle {
    fn default() -> Self {
        Self {
            normal_00: Style::new().bg(Color::Rgb(57, 57, 57)).fg(Color::White),
            normal_01: Style::new().bg(Color::Rgb(60, 60, 60)).fg(Color::White),
            normal_10: Style::new().bg(Color::Rgb(67, 67, 67)).fg(Color::White),
            normal_11: Style::new().bg(Color::Rgb(70, 70, 70)).fg(Color::White),
            primary_selection: Style::new().bg(Color::LightBlue).fg(Color::Black),
            secondary_selection: Style::new().bg(Color::Blue).fg(Color::Blue),
            yanked: Style::new().bg(Color::Green).fg(Color::Black),
        }
    }
}

#[derive(Debug, Clone)]
struct CsvTableWrapper {
    visible_cols: usize,
    visible_rows: usize,
    cell_height_wanted: u16,
    cell_width_wanted: u16,
    cell_height: u16,
    cell_width: u16,
    style: CsvTableWidgetStyle,
    top_left_cell_location: CellLocation,
    csv_table: CsvTable,
    selection: Selection,
    selection_yanked: Option<Selection>,
}

impl Default for CsvTableWrapper {
    fn default() -> Self {
        Self {
            visible_cols: 5,
            visible_rows: 20,
            cell_height_wanted: 1,
            cell_width_wanted: 25,
            cell_height: 0,
            cell_width: 0,
            style: Default::default(),
            top_left_cell_location: Default::default(),
            csv_table: Default::default(),
            selection: Default::default(),
            selection_yanked: Default::default(),
        }
    }
}

impl CsvTableWrapper {
    fn move_selection(&mut self, direction: MoveDirection, n: usize) {
        let n = n as isize;
        let delta = match direction {
            MoveDirection::Left => CellLocationDelta { x: -n, y: 0 },
            MoveDirection::Down => CellLocationDelta { x: 0, y: n },
            MoveDirection::Up => CellLocationDelta { x: 0, y: -n },
            MoveDirection::Right => CellLocationDelta { x: n, y: 0 },
        };
        self.selection.primary = self.selection.primary + delta;
    }

    fn recalculate_dimensions(&mut self, available_cols: u16, available_rows: u16) {
        self.visible_rows = (available_rows / self.cell_height_wanted) as usize;
        if self.visible_rows == 0 {
            self.visible_rows = if available_rows == 0 { 0 } else { 1 };
            self.cell_height = available_rows;
        } else {
            self.cell_height = self.cell_height_wanted + available_rows % self.cell_height_wanted;
        }

        self.visible_cols = (available_cols / self.cell_width_wanted) as usize;
        if self.visible_cols == 0 {
            self.visible_cols = if available_cols == 0 { 0 } else { 1 };
            self.cell_width = available_cols;
        } else {
            self.cell_width = self.cell_width_wanted + available_cols % self.cell_width_wanted;
        }
    }
}

/// https://ratatui.rs/recipes/layout/grid/
impl Widget for &CsvTableWrapper {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let CsvTableWrapper {
            visible_cols: cols,
            visible_rows: rows,
            cell_height,
            cell_width,
            style,
            top_left_cell_location,
            csv_table,
            selection,
            selection_yanked,
            ..
        } = self;

        let CsvTableWidgetStyle {
            normal_00,
            normal_01,
            normal_10,
            normal_11,
            primary_selection,
            secondary_selection,
            yanked,
        } = style;

        let Selection { selected, primary } = selection;
        let col_constraints = (0..*cols).map(|_| Constraint::Length(*cell_width));
        let row_constraints = (0..*rows).map(|_| Constraint::Length(*cell_height));
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
            let cell_location = *top_left_cell_location + CellLocation { row, col };
            let text = csv_table.get(cell_location).unwrap_or_default();
            let style = if *primary == cell_location {
                primary_selection
            } else if selected.contains(&cell_location) {
                secondary_selection
            } else if let Some(selection) = &selection_yanked
                && (selection.primary == cell_location
                    || selection.selected.contains(&cell_location))
            {
                yanked
            } else {
                match (row % 2, col % 2) {
                    (0, 0) => normal_00,
                    (0, 1) => normal_01,
                    (1, 0) => normal_10,
                    (1, 1) => normal_11,
                    _ => unreachable!(),
                }
            };
            Paragraph::new(text).style(*style).render(cell, buf);
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

#[derive(Debug, Clone, Default)]
struct Selection {
    selected: Vec<CellLocation>,
    primary: CellLocation,
}

#[derive(Debug, Clone, Default)]
struct Yank {
    content: Vec<Vec<Option<String>>>,
}

impl Yank {
    fn new(content: Vec<Vec<Option<String>>>) -> Self {
        Self { content }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MoveDirection {
    Left,
    Down,
    Up,
    Right,
}
