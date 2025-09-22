pub mod content;
pub(crate) mod symbols;

use clap::Parser;
use color_eyre::{Result, eyre::eyre};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Position, Rect},
    style::{Color, Style, Stylize},
    widgets::{Block, Clear, Paragraph, Widget},
};
use regex::Regex;
use std::{borrow::Cow, cell::LazyCell, fmt::Display, path::PathBuf, str::FromStr};

use crate::content::{CellLocation, CellLocationDelta, CsvTable};

const LOGO: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/logo.txt"));

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
        self.terminal
            .draw(|frame| frame.render_widget(SplashScreen, frame.area()))?;

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
                self.state.input = InputState::default();
            }
            return Ok(());
        }
        match &self.state.input {
            InputState::Normal { .. } => match (key.modifiers, key.code) {
                (_, KeyCode::Char(':')) => {
                    self.state.input = InputState::Console(Console {
                        mode: ConsoleBarMode::Console,
                        content: String::default(),
                    })
                }
                _ if self.state.table.is_some() => {
                    let res = self.handle_table_key_input(key);
                    if res.is_err() || matches!(res, Ok(false)) {
                        self.state.input = Default::default();
                        res?;
                    }
                }
                _ => {}
            },
            InputState::Console(_) => self.handle_console_input(key)?,
        }
        Ok(())
    }

    fn handle_table_key_input(&mut self, key: KeyEvent) -> Result<bool> {
        let InputState::Normal(Normal {
            combo,
            collect_all,
            input_buffer,
        }) = &mut self.state.input
        else {
            unreachable!();
        };

        if let KeyCode::Char(c) = key.code
            && (c.is_ascii_digit()
                || (input_buffer.is_empty() && (c == '+' || c == '-'))
                || (*collect_all && c.is_ascii_uppercase() || c.is_ascii_digit()))
        {
            input_buffer.push(c);
            return Ok(true);
        }

        let table = self.state.table.as_mut().unwrap();
        match (key.modifiers, key.code, *combo) {
            (_, KeyCode::Char('c' | 'z'), Some(Combo::View)) => {
                table.center_primary_selection();
            }
            (_, KeyCode::Char('g'), Some(Combo::Goto)) => {
                if input_buffer.is_empty() {
                    table.move_selection_to(CellLocation { row: 0, col: 0 });
                } else {
                    let location_id = CsvJump::from_str(input_buffer)?;
                    let location = location_id.combine(table.selection.primary);
                    table.move_selection_to(location);
                }
            }
            (_, KeyCode::Char('z'), None) => {
                *combo = Some(Combo::View);
                return Ok(true);
            }
            (_, KeyCode::Char('g'), None) => {
                *combo = Some(Combo::Goto);
                *collect_all = true;
                return Ok(true);
            }
            (_, KeyCode::Char('H'), None) => {
                table.selection.selected = Vec::new();
                table.move_selection(MoveDirection::Left, table.visible_cols / 2);
            }
            (KeyModifiers::CONTROL, KeyCode::Char('d'), None) | (_, KeyCode::Char('J'), None) => {
                table.selection.selected = Vec::new();
                table.move_selection(MoveDirection::Down, table.visible_rows / 2);
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u'), None) | (_, KeyCode::Char('K'), None) => {
                table.selection.selected = Vec::new();
                table.move_selection(MoveDirection::Up, table.visible_rows / 2);
            }
            (_, KeyCode::Char('L'), None) => {
                table.selection.selected = Vec::new();
                table.move_selection(MoveDirection::Right, table.visible_cols / 2);
            }
            (_, KeyCode::Char('h'), None) => {
                let num = input_buffer.parse().unwrap_or(1);
                table.move_selection(MoveDirection::Left, num);
            }
            (_, KeyCode::Char('j'), None) => {
                let num = input_buffer.parse().unwrap_or(1);
                table.move_selection(MoveDirection::Down, num);
            }
            (_, KeyCode::Char('k'), None) => {
                let num = input_buffer.parse().unwrap_or(1);
                table.move_selection(MoveDirection::Up, num);
            }
            (_, KeyCode::Char('l'), None) => {
                let num = input_buffer.parse().unwrap_or(1);
                table.move_selection(MoveDirection::Right, num);
            }
            (_, KeyCode::Char('i'), None) => {
                let content = table
                    .csv_table
                    .get(table.selection.primary)
                    .unwrap_or_default();
                self.state.input = InputState::Console(Console {
                    mode: ConsoleBarMode::CellInput,
                    content: content.to_owned(),
                });
                return Ok(true);
            }
            (_, KeyCode::Char('c'), None) => {
                self.state.input = InputState::Console(Console {
                    mode: ConsoleBarMode::CellInput,
                    content: Default::default(),
                });
                return Ok(true);
            }
            (_, KeyCode::Char('y'), None) => {
                // TODO: implement for rectangle selections
                let content = table
                    .csv_table
                    .get(table.selection.primary)
                    .map(ToOwned::to_owned);
                let content = vec![vec![content]];
                table.selection_yanked = Some(table.selection.clone());
                self.state.yank = Some(Yank::new(content))
            }
            (_, KeyCode::Char('d'), None) => {
                // TODO: implement for rectangle selections
                let content = table
                    .csv_table
                    .get(table.selection.primary)
                    .map(ToOwned::to_owned);
                let content = vec![vec![content]];
                table.csv_table.set(table.selection.primary, None);
                self.state.yank = Some(Yank::new(content))
            }
            (_, KeyCode::Char('p'), None) => {
                // TODO: implement for rectangle selections
                if let Some(Yank { content, .. }) = &self.state.yank {
                    table
                        .csv_table
                        .set(table.selection.primary, content[0][0].clone());
                    table.selection_yanked = None;
                }
            }
            _ => {}
        }
        Ok(false)
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

                self.state.input = InputState::default();
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
            Layout::vertical([Constraint::Percentage(100), Constraint::Min(1)]).areas(frame.area());
        frame.render_widget(Block::new(), main_area);
        if let Some(table) = &mut self.table {
            table.recalculate_dimensions(main_area.width, main_area.height);
            frame.render_widget(&*table, main_area);
        } else {
            frame.render_widget(SplashScreen, main_area);
        }
        let [main_console, status] =
            Layout::horizontal([Constraint::Percentage(100), Constraint::Min(22)])
                .areas(console_bar);

        if let InputState::Console(console) = &self.input {
            frame.render_widget(console, main_console);
        } else if let Some(console_message) = &self.console_message {
            frame.render_widget(console_message, main_console);
        }

        frame.render_widget(StatusWidget { state: self }, status);
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
            yanked: Style::new().fg(Color::Green),
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
        self.selection.primary += delta;
        self.ensure_selection_in_view();
    }

    fn move_selection_to(&mut self, location: CellLocation) {
        self.selection.primary = location;
        self.ensure_selection_in_view();
    }

    fn ensure_selection_in_view(&mut self) {
        let sel = self.selection.primary;

        if sel.col < self.top_left_cell_location.col {
            self.top_left_cell_location.col = sel.col;
        } else if sel.col >= self.top_left_cell_location.col + self.visible_cols {
            self.top_left_cell_location.col = sel.col - self.visible_cols + 1;
        }

        if sel.row < self.top_left_cell_location.row {
            self.top_left_cell_location.row = sel.row;
        } else if sel.row >= self.top_left_cell_location.row + self.visible_rows {
            self.top_left_cell_location.row = sel.row - self.visible_rows + 1;
        }
    }

    pub fn center_primary_selection(&mut self) {
        self.top_left_cell_location = self.selection.primary
            - CellLocationDelta {
                x: (self.visible_cols / 2) as isize,
                y: (self.visible_rows / 2) as isize,
            }
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
            } else {
                match (row % 2, col % 2) {
                    (0, 0) => normal_00,
                    (0, 1) => normal_01,
                    (1, 0) => normal_10,
                    (1, 1) => normal_11,
                    _ => unreachable!(),
                }
            };

            let area = if let Some(selection) = &selection_yanked
                && (selection.primary == cell_location
                    || selection.selected.contains(&cell_location))
            {
                let [left, main, right] = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(1), // links
                        Constraint::Min(0),    // Mitte: Text, flexibel
                        Constraint::Length(1), // rechts
                    ])
                    .areas(cell);
                let yank_style = style.patch(*yanked);
                // Left border
                for y in 0..left.height {
                    buf.cell_mut(Position::new(left.x, left.y + y))
                        .unwrap()
                        .set_symbol(symbols::HALF_BLOCK_LEFT)
                        .set_style(yank_style);
                }

                // Right border
                for y in 0..right.height {
                    buf.cell_mut(Position::new(right.x, right.y + y))
                        .unwrap()
                        .set_symbol(symbols::HALF_BLOCK_RIGHT)
                        .set_style(yank_style);
                }
                main
            } else {
                cell
            };

            Paragraph::new(text)
                .alignment(Alignment::Center)
                .style(*style)
                .render(area, buf);
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
        Clear.render(area, buf);
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
        Clear.render(area, buf);
        let paragraph = Paragraph::new(format!("{prefix}{content}"));
        paragraph.render(area, buf);
    }
}

#[derive(Clone, Debug)]
struct SplashScreen;

impl Widget for SplashScreen {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let lines: Vec<&str> = LOGO.lines().collect();
        let logo_height = lines.len() as u16;

        // Vertikale Zentrierung
        let start_y = if area.height > logo_height {
            area.y + (area.height - logo_height) / 2
        } else {
            area.y
        };

        // Paragraph für das ganze Logo
        let paragraph = Paragraph::new(LOGO).alignment(Alignment::Center);

        // Paragraph rendern direkt auf Buffer
        let logo_area = Rect {
            x: area.x,
            y: start_y,
            width: area.width,
            height: logo_height.min(area.height),
        };

        paragraph.render(logo_area, buf);
    }
}

#[derive(Clone, Debug)]
enum InputState {
    Normal(Normal),
    Console(Console),
}

impl Default for InputState {
    fn default() -> Self {
        Self::Normal(Normal::default())
    }
}

#[derive(Clone, Debug, Default)]
struct Normal {
    combo: Option<Combo>,
    collect_all: bool,
    input_buffer: String,
}

struct StatusWidget<'a> {
    state: &'a AppState,
}

impl<'a> Widget for StatusWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let StatusWidget { state } = self;
        let [buffer_area, combo_area, _, mode_area, coords_area] = Layout::horizontal([
            Constraint::Length(9),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(8),
        ])
        .areas(area);

        let (mode, buffer_str, combo_str) = match &state.input {
            InputState::Normal(Normal {
                combo,
                input_buffer,
                ..
            }) => (
                "NOR",
                Some(input_buffer),
                combo.as_ref().map(ToString::to_string),
            ),
            InputState::Console(Console { mode, .. }) => match mode {
                ConsoleBarMode::Console => ("CON", None, None),
                ConsoleBarMode::CellInput => ("INS", None, None),
            },
        };

        if let Some(buffer_str) = buffer_str {
            Paragraph::new(buffer_str.as_str())
                .alignment(Alignment::Right)
                .render(buffer_area, buf);
        }

        if let Some(combo_str) = combo_str {
            Paragraph::new(combo_str.as_str()).render(combo_area, buf);
        }

        Paragraph::new(mode).render(mode_area, buf);

        if let Some(table) = &state.table {
            Paragraph::new(table.selection.primary.to_string())
                .alignment(Alignment::Right)
                .render(coords_area, buf);
        };
    }
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
#[command(version, about, long_about = "Minimalistic Csv Editor")]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Combo {
    View,
    Goto,
}

impl Display for Combo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Combo::View => "v",
            Combo::Goto => "g",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CsvJump {
    sign: Option<isize>,
    row: Option<usize>,
    col: Option<usize>,
}

impl CsvJump {
    #[must_use]
    fn combine(self, location: CellLocation) -> CellLocation {
        let Some(sign) = self.sign else {
            return CellLocation {
                row: self.row.unwrap_or(location.row),
                col: self.col.unwrap_or(location.col),
            };
        };

        let row = if let Some(r) = self.row {
            if sign == -1 {
                location.row.saturating_sub(r)
            } else {
                location.row + r
            }
        } else {
            location.row
        };
        let col = if let Some(c) = self.col {
            if sign == -1 {
                location.col.saturating_sub(c)
            } else {
                location.col + c
            }
        } else {
            location.col
        };
        CellLocation { row, col }
    }
}

impl FromStr for CsvJump {
    type Err = color_eyre::eyre::Report;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        thread_local! {
            static RE: LazyCell<Regex> = LazyCell::new(|| Regex::new(r#"^(?P<sign>[+-])?(?P<col>[[:alpha:]]+)?(?P<row>\d+)?$"#).unwrap());
        }
        let Some(caps) = RE.with(|i| i.captures(s)) else {
            return Err(eyre!("Not a valid location id!"));
        };

        let sign = match caps.name("sign").map(|s| s.as_str()) {
            Some("+") => Some(1),
            Some("-") => Some(-1),
            _ => None,
        };

        let row = caps
            .name("row")
            .map(|row| row.as_str().parse::<usize>().unwrap().saturating_sub(1));
        let col = caps.name("col").map(|col| {
            let mut result = 0usize;
            for c in col.as_str().chars() {
                assert!(c.is_ascii_alphabetic());
                let val = (c.to_ascii_uppercase() as u8 - b'A') as usize;
                result = result * 26 + val + 1;
            }
            result - 1
        });
        if row.is_none() && col.is_none() {
            return Err(eyre!("Emtpy location id!"));
        }
        Ok(Self { sign, row, col })
    }
}
