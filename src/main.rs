mod buffer;
pub(crate) mod color_ext;
mod content;
pub(crate) mod symbols;

use clap::Parser;
use color_eyre::{
    Result,
    eyre::{bail, eyre},
};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Position, Rect},
    style::{Color, Style, Stylize},
    widgets::{Block, Clear, Paragraph, Widget},
};
use regex::Regex;
use std::{
    borrow::Cow,
    cell::LazyCell,
    fmt::{Debug, Display},
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::{
    buffer::{CsvBuffer, LoadOption},
    color_ext::ColorExt,
    content::CellLocation,
};

const LOGO: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/logo.txt"));
const ROW_LABEL_WIDTH: u16 = 4;

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
    running: bool,
    input: InputState,
    console_message: Option<ConsoleMessage>,
    table: Option<CsvBuffer>,
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

        if let Err(err) = self.try_init(args) {
            self.state.console_message = Some(ConsoleMessage::error(format!("{err}")));
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
            InputState::Main { .. } => match (key.modifiers, key.code) {
                (_, KeyCode::Char(':')) => {
                    self.state.input = InputState::Console(InputModeConsole {
                        mode: ConsoleBarMode::Console,
                        content: String::default(),
                    })
                }
                _ if self.state.table.is_some() => {
                    let res = self.handle_table_key_input(key);
                    if res.is_err() {
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

    fn handle_table_key_input(&mut self, key: KeyEvent) -> Result<()> {
        let InputState::Main(InputModeMain {
            mode,
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
            return Ok(());
        }

        let mut keep_combo = false;

        let table = self.state.table.as_mut().unwrap();
        match (key.modifiers, key.code, *combo) {
            // View
            (_, KeyCode::Char('c' | 'z'), Some(Combo::View)) => {
                table.center_primary_selection();
            }
            (_, KeyCode::Char('h'), Some(Combo::View)) => {
                let num = input_buffer.parse().unwrap_or(1);
                table.move_view(MoveDirection::Left, num);
            }
            (_, KeyCode::Char('j'), Some(Combo::View)) => {
                let num = input_buffer.parse().unwrap_or(1);
                table.move_view(MoveDirection::Down, num);
            }
            (_, KeyCode::Char('k'), Some(Combo::View)) => {
                let num = input_buffer.parse().unwrap_or(1);
                table.move_view(MoveDirection::Up, num);
            }
            (_, KeyCode::Char('l'), Some(Combo::View)) => {
                let num = input_buffer.parse().unwrap_or(1);
                table.move_view(MoveDirection::Right, num);
            }
            // Goto
            (_, KeyCode::Char('g'), Some(Combo::Goto)) => {
                if input_buffer.is_empty() {
                    table.move_selection_to(CellLocation { row: 0, col: 0 });
                } else {
                    let location_id = CsvJump::from_str(input_buffer)?;
                    let location = location_id.combine(table.selection.primary);
                    table.move_selection_to(location);
                }
            }
            (_, KeyCode::Char('h'), Some(Combo::Goto)) => {
                table.move_selection_to(CellLocation {
                    row: table.selection.primary.row,
                    col: 0,
                });
            }
            (_, KeyCode::Char('k'), Some(Combo::Goto)) => {
                table.move_selection_to(CellLocation {
                    row: 0,
                    col: table.selection.primary.col,
                });
            }
            // No mode
            (_, KeyCode::Char('z'), None) => {
                *combo = Some(Combo::View);
                keep_combo = true;
            }
            (_, KeyCode::Char('g'), None) => {
                *combo = Some(Combo::Goto);
                *collect_all = true;
                keep_combo = true;
            }
            (_, KeyCode::Char('v'), None) => {
                if *mode == MainMode::Normal {
                    table.selection.opposite = Some(table.selection.primary);
                    *mode = MainMode::Visual
                } else {
                    table.selection.opposite = None;
                    *mode = MainMode::Normal
                };
            }
            (_, KeyCode::Char('H'), None) => {
                table.move_selection(MoveDirection::Left, table.visible_cols / 2);
            }
            (KeyModifiers::CONTROL, KeyCode::Char('d'), None) | (_, KeyCode::Char('J'), None) => {
                table.move_selection(MoveDirection::Down, table.visible_rows / 2);
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u'), None) | (_, KeyCode::Char('K'), None) => {
                table.move_selection(MoveDirection::Up, table.visible_rows / 2);
            }
            (_, KeyCode::Char('L'), None) => {
                table.move_selection(MoveDirection::Right, table.visible_cols / 2);
            }
            (_, KeyCode::Char('h') | KeyCode::Left, None) => {
                let num = input_buffer.parse().unwrap_or(1);
                table.move_selection(MoveDirection::Left, num);
            }
            (_, KeyCode::Char('j') | KeyCode::Down, None) => {
                let num = input_buffer.parse().unwrap_or(1);
                table.move_selection(MoveDirection::Down, num);
            }
            (_, KeyCode::Char('k') | KeyCode::Up, None) => {
                let num = input_buffer.parse().unwrap_or(1);
                table.move_selection(MoveDirection::Up, num);
            }
            (_, KeyCode::Char('l') | KeyCode::Right, None) => {
                let num = input_buffer.parse().unwrap_or(1);
                table.move_selection(MoveDirection::Right, num);
            }
            (_, KeyCode::Char('i'), None) => {
                let content = table
                    .csv_table
                    .get(table.selection.primary)
                    .unwrap_or_default();
                self.state.input = InputState::Console(InputModeConsole {
                    mode: ConsoleBarMode::CellInput,
                    content: content.to_owned(),
                });
            }
            (_, KeyCode::Char('c'), None) => {
                self.state.input = InputState::Console(InputModeConsole {
                    mode: ConsoleBarMode::CellInput,
                    content: Default::default(),
                });
            }
            (_, KeyCode::Char('Y'), None) => table.selection_yanked = None,
            (_, KeyCode::Char('y'), None) => {
                let Selection { primary, opposite } = table.selection;
                let yank = if let Some(opposite) = opposite {
                    let content = primary
                        .rect_iter(opposite)
                        .map(|c| table.csv_table.get(c).map(ToOwned::to_owned))
                        .collect();
                    Yank::Rectangle {
                        cols: primary.get_column_count(opposite),
                        content,
                    }
                } else {
                    let content = table.csv_table.get(primary).map(ToOwned::to_owned);
                    Yank::Single(content)
                };
                table.selection_yanked = Some(table.selection);
                self.state.yank = Some(yank);
                table.selection.opposite = None;
                *mode = MainMode::Normal;
            }
            (_, KeyCode::Char('d'), None) => {
                let Selection { primary, opposite } = table.selection;
                let yank = if let Some(opposite) = opposite {
                    let mut content = Vec::default();
                    for cell in primary.rect_iter(opposite) {
                        content.push(table.csv_table.get(cell).map(ToOwned::to_owned));
                        table.csv_table.set(cell, None);
                    }
                    Yank::Rectangle {
                        cols: primary.get_column_count(opposite),
                        content,
                    }
                } else {
                    let content = table.csv_table.get(primary).map(ToOwned::to_owned);
                    table.csv_table.set(primary, None);
                    Yank::Single(content)
                };
                table.selection_yanked = None;
                self.state.yank = Some(yank);
                table.selection.opposite = None;
                *mode = MainMode::Normal;
            }
            (_, KeyCode::Char('p'), None) => {
                let Selection { primary, opposite } = table.selection;
                if let Some(yank) = &self.state.yank {
                    match yank {
                        Yank::Single(single) => {
                            if let Some(opposite) = opposite {
                                for cell in primary.rect_iter(opposite) {
                                    table.csv_table.set(cell, single.clone());
                                }
                            } else {
                                table.csv_table.set(primary, single.clone());
                            }
                        }
                        Yank::Rectangle { cols, content } => {
                            for (content, dst) in
                                content.iter().zip(primary.rect_iter(CellLocation {
                                    row: primary.row + content.len() / cols - 1,
                                    col: primary.col + cols - 1,
                                }))
                            {
                                table.csv_table.set(dst, content.clone());
                            }
                        }
                    }
                    *mode = MainMode::Normal;
                }
            }
            _ => {}
        }
        if let InputState::Main(InputModeMain {
            combo,
            collect_all,
            input_buffer,
            ..
        }) = &mut self.state.input
            && !keep_combo
        {
            *combo = Default::default();
            *collect_all = Default::default();
            *input_buffer = Default::default();
        }
        Ok(())
    }

    fn handle_console_input(&mut self, key: KeyEvent) -> Result<()> {
        let InputState::Console(InputModeConsole { mode, content }) = &mut self.state.input else {
            unreachable!();
        };
        match (key.modifiers, key.code) {
            (_, KeyCode::Enter) => {
                let content = content.clone();
                let res = match mode {
                    ConsoleBarMode::Console => self.try_execute_command(&content),
                    ConsoleBarMode::CellInput => {
                        if let Some(table) = &mut self.state.table {
                            table.csv_table.set(table.selection.primary, Some(content));
                        }
                        Ok(())
                    }
                };
                self.state.input = InputState::default();
                res?;
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
        let command_split = command
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        match &command_split[..] {
            ["q!" | "quit!", ..] => {
                self.quit();
            }
            ["wq" | "x" | "write-quit", rest @ ..] => {
                let file = rest.first().map(|f| PathBuf::from_str(f)).transpose()?;
                if let Some(table) = &mut self.state.table {
                    table.save(file, false)?;
                };
                self.quit();
            }
            ["wq!" | "x!" | "write-quit!", rest @ ..] => {
                if let Some(table) = &mut self.state.table {
                    let file = rest.first().map(|f| PathBuf::from_str(f)).transpose()?;
                    table.save(file, true)?;
                };
                self.quit();
            }
            ["q" | "quit", ..] => {
                let Some(table) = &self.state.table else {
                    self.quit();
                    return Ok(());
                };
                if table.is_dirty() {
                    bail!("There are unsaved changes! Use `quit!` to force quit!",);
                }
                self.quit();
            }
            ["bc" | "buffer-close", ..] => {
                let Some(table) = &self.state.table else {
                    self.state.table = None;
                    return Ok(());
                };
                if table.is_dirty() {
                    bail!(
                        "There are unsaved changes! Use `buffer-close!` to force closing buffer!",
                    );
                }
                self.state.table = None;
            }
            ["o" | "open", file, rest @ ..] => {
                let delimiter = rest.first().and_then(|c| c.chars().next()).map(|c| c as u8);
                let res = CsvBuffer::load(LoadOption::File(PathBuf::from(file)), delimiter);
                match res {
                    Ok(t) => self.state.table = Some(t),
                    Err(err) => {
                        self.state.console_message = Some(ConsoleMessage::error(format!("{err}")));
                    }
                }
            }
            ["n" | "new", ..] => {
                if self.state.table.is_none() {
                    self.state.table = Some(CsvBuffer::default())
                }
            }
            ["bc!" | "buffer-close!", ..] => {
                self.state.table = None;
            }
            [c, ..] => {
                let handled = if self.state.table.is_some() {
                    self.handle_table_commands(&command_split)?
                } else {
                    false
                };
                if !handled {
                    bail!("Unknown command: {c}");
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_table_commands(&mut self, command: &[&str]) -> Result<bool> {
        let Some(table) = &mut self.state.table else {
            unreachable!();
        };

        match command {
            ["w" | "write", rest @ ..] => {
                let file = rest.first().map(|f| PathBuf::from_str(f)).transpose()?;
                let saved = table.save(file, false)?;
                self.state.console_message = Some(ConsoleMessage::new(format!(
                    "{} written!",
                    saved.to_string_lossy()
                )))
            }
            ["w!" | "write!", rest @ ..] => {
                let file = rest.first().map(|f| PathBuf::from_str(f)).transpose()?;
                let saved = table.save(file, true)?;
                self.state.console_message = Some(ConsoleMessage::new(format!(
                    "{} written!",
                    saved.to_string_lossy()
                )))
            }
            ["delimiter"] => {
                let message = match table.csv_table.delimiter {
                    Some(b'\t') => r"\t".to_string(),
                    Some(delim) => (delim as char).to_string(),
                    None => "unset".to_string(),
                };
                self.state.console_message = Some(ConsoleMessage::new(message));
            }
            ["delimiter", d, ..] => {
                table.csv_table.delimiter = match *d {
                    "unset" => None,
                    r"\t" => Some(b'\t'),
                    s if s.len() == 1 => Some(s.as_bytes()[0]),
                    _ => table.csv_table.delimiter,
                };
            }
            ["save-path", ..] => {
                let message = table
                    .file
                    .as_deref()
                    .map(Path::to_string_lossy)
                    .unwrap_or("No save path set!".into());
                self.state.console_message = Some(ConsoleMessage::new(message.into_owned()))
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn try_init(&mut self, args: Args) -> color_eyre::Result<()> {
        let Args {
            delimiter,
            file,
            stdin,
        } = args;
        let load_option = if let Some(file) = file {
            LoadOption::File(file)
        } else if stdin {
            LoadOption::Stdin
        } else {
            return Ok(());
        };
        let table = CsvBuffer::load(load_option, delimiter.map(|d| d as u8))?;
        self.state.table = Some(table);
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
        let [column_labels_area, main_area, console_bar] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Percentage(100),
            Constraint::Min(1),
        ])
        .areas(frame.area());

        frame.render_widget(Block::new(), main_area);
        if let Some(table) = &mut self.table {
            let [corner, col_labels_area] = Layout::horizontal([
                Constraint::Min(ROW_LABEL_WIDTH),
                Constraint::Percentage(100),
            ])
            .areas(column_labels_area);
            let [row_labels_area, main_area] = Layout::horizontal([
                Constraint::Min(ROW_LABEL_WIDTH),
                Constraint::Percentage(100),
            ])
            .areas(main_area);

            table.recalculate_dimensions(main_area.width, main_area.height);

            // Render labels: Could also use one widget with the whole area
            Block::new()
                .style(table.style.label_normal)
                .render(corner, frame.buffer_mut());
            frame.render_widget(ColLabelsWidget(table), col_labels_area);
            frame.render_widget(RowLabelsWidget(table), row_labels_area);

            frame.render_widget(MainTableWidget(table), main_area);
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

        frame.render_widget(StatusWidget(self), status);
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
struct CsvTableWidgetStyle {
    normal_00: Style,
    normal_01: Style,
    normal_10: Style,
    normal_11: Style,
    primary_selection: Style,
    yanked: Style,
    label_normal: Style,
    label_primary_selection: Style,
}

impl Default for CsvTableWidgetStyle {
    fn default() -> Self {
        Self {
            normal_00: Style::new().bg(Color::Rgb(30, 30, 30)).fg(Color::White),
            normal_01: Style::new().bg(Color::Rgb(31, 31, 31)).fg(Color::White),
            normal_10: Style::new().bg(Color::Rgb(39, 39, 39)).fg(Color::White),
            normal_11: Style::new().bg(Color::Rgb(41, 41, 41)).fg(Color::White),
            primary_selection: Style::new().bg(Color::LightBlue).fg(Color::Black),
            yanked: Style::new().fg(Color::Green),
            label_normal: Style::new().bg(Color::Black).fg(Color::Rgb(160, 160, 160)),
            label_primary_selection: Style::new().bg(Color::Black).fg(Color::LightBlue),
        }
    }
}

#[derive(Clone, Debug)]
struct MainTableWidget<'a>(&'a CsvBuffer);

/// https://ratatui.rs/recipes/layout/grid/
impl Widget for MainTableWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let CsvBuffer {
            visible_cols,
            visible_rows,
            cell_height,
            cell_width,
            style,
            top_left_cell_location,
            csv_table,
            selection,
            selection_yanked,
            ..
        } = self.0;

        let CsvTableWidgetStyle {
            normal_00,
            normal_01,
            normal_10,
            normal_11,
            primary_selection,
            yanked,
            ..
        } = style;

        let Selection { opposite, primary } = selection;
        let col_constraints = (0..*visible_cols).map(|_| Constraint::Length(*cell_width));
        let row_constraints = (0..*visible_rows).map(|_| Constraint::Length(*cell_height));
        let horizontal = Layout::horizontal(col_constraints).spacing(0);
        let vertical = Layout::vertical(row_constraints).spacing(0);

        let rows = vertical.split(area);
        let cells = rows.iter().flat_map(|&row| horizontal.split(row).to_vec());

        // Possible in new version
        // let cells = area
        //     .layout_vec(&vertical)
        //     .iter()
        //     .flat_map(|row| row.layout_vec(&horizontal));

        for (i, cell) in cells.enumerate() {
            let row_view = i / visible_cols;
            let col_view = i % visible_cols;
            let cell_location @ CellLocation { col, .. } = *top_left_cell_location
                + CellLocation {
                    row: row_view,
                    col: col_view,
                };
            let text = csv_table.get(cell_location).unwrap_or_default();

            let normal = match (row_view % 2, col_view % 2) {
                (0, 0) => normal_00,
                (0, 1) => normal_01,
                (1, 0) => normal_10,
                (1, 1) => normal_11,
                _ => unreachable!(),
            };

            let is_yanked = selection_yanked
                .map(|Selection { primary, opposite }| {
                    opposite
                        .map(|o| cell_location.in_rect(primary, o))
                        .unwrap_or(cell_location == primary)
                })
                .unwrap_or_default();

            let style = if *primary == cell_location {
                *primary_selection
            } else if opposite
                .map(|opposite| cell_location.in_rect(*primary, opposite))
                .unwrap_or_default()
                && let Some(primary_bg) = primary_selection.bg
                && let Some(normal_bg) = normal.bg
            {
                let mut style = Style::new().bg(primary_bg.mix(normal_bg, 0.7, false).mix(
                    Color::Rgb(0, 0, 0),
                    0.1,
                    false,
                ));
                if let Some(primary_fg) = primary_selection.fg {
                    style = style.fg(primary_fg);
                }
                style
            } else if is_yanked
                && let Some(Selection { primary, opposite }) = selection_yanked
                && opposite
                    .map(|o| cell_location.in_rect(*primary, o))
                    .unwrap_or(cell_location == *primary)
            {
                let bg = yanked.bg.or(yanked.fg).unwrap_or(Color::LightGreen);
                let bg = normal.bg.map(|n| bg.mix(n, 0.9, false)).unwrap_or(bg);
                normal.bg(bg)
            } else {
                *normal
            };

            // Border for yanked left and right
            let area = if is_yanked
                && let Some(Selection {
                    primary:
                        CellLocation {
                            col: col_primary, ..
                        },
                    opposite,
                }) = &selection_yanked
                && (*col_primary == col || opposite.map(|o| o.col == col).unwrap_or_default())
            {
                let (left, main, right) = if let Some(CellLocation {
                    col: col_opposite, ..
                }) = opposite
                {
                    if *col_primary == *col_opposite {
                        let [left, main, right] = Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints([
                                Constraint::Length(1),
                                Constraint::Min(0),
                                Constraint::Length(1),
                            ])
                            .areas(cell);
                        (Some(left), main, Some(right))
                    } else if col == (*col_primary).min(*col_opposite) {
                        let [left, main] = Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints([Constraint::Length(1), Constraint::Min(0)])
                            .areas(cell);
                        (Some(left), main, None)
                    } else {
                        let [main, right] = Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints([Constraint::Min(0), Constraint::Length(1)])
                            .areas(cell);
                        (None, main, Some(right))
                    }
                } else {
                    let [left, main, right] = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Length(1),
                            Constraint::Min(0),
                            Constraint::Length(1),
                        ])
                        .areas(cell);
                    (Some(left), main, Some(right))
                };

                let yank_style = style.patch(*yanked);
                if let Some(left) = left {
                    // Left border
                    for y in 0..left.height {
                        buf.cell_mut(Position::new(left.x, left.y + y))
                            .unwrap()
                            .set_symbol(symbols::HALF_BLOCK_LEFT)
                            .set_style(yank_style);
                    }
                }

                if let Some(right) = right {
                    // Right border
                    for y in 0..right.height {
                        buf.cell_mut(Position::new(right.x, right.y + y))
                            .unwrap()
                            .set_symbol(symbols::HALF_BLOCK_RIGHT)
                            .set_style(yank_style);
                    }
                }
                main
            } else {
                cell
            };

            Paragraph::new(text)
                .alignment(Alignment::Center)
                .style(style)
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
    pub(crate) fn new(message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            message: message.into(),
            ..Default::default()
        }
    }

    #[expect(unused)]
    pub fn severity(self, severity: Severity) -> Self {
        Self { severity, ..self }
    }

    pub(crate) fn error(message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            message: message.into(),
            severity: Severity::Error,
        }
    }

    #[expect(unused)]
    pub(crate) fn warning(message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            message: message.into(),
            severity: Severity::Warning,
        }
    }

    #[expect(unused)]
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
struct InputModeConsole {
    mode: ConsoleBarMode,
    content: String,
}

impl Widget for &InputModeConsole {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let InputModeConsole { mode, content } = self;
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
struct ColLabelsWidget<'a>(&'a CsvBuffer);

impl<'a> Widget for ColLabelsWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let ColLabelsWidget(CsvBuffer {
            visible_cols,
            cell_width,
            style,
            top_left_cell_location,
            selection,
            ..
        }) = self;

        let CellLocation { col: col_left, .. } = top_left_cell_location;
        let col_constraints = (0..*visible_cols).map(|_| Constraint::Length(*cell_width));
        let labels = Layout::horizontal(col_constraints).spacing(0).split(area);

        for col_label in 0..*visible_cols {
            let col = col_left + col_label;
            let style = if selection.primary.col == col {
                style.label_primary_selection
            } else {
                style.label_normal
            };
            Paragraph::new(CellLocation::col_index_to_id(col))
                .style(style)
                .alignment(Alignment::Center)
                .render(labels[col_label], buf);
        }
    }
}
#[derive(Clone, Debug)]

struct RowLabelsWidget<'a>(&'a CsvBuffer);

impl<'a> Widget for RowLabelsWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let RowLabelsWidget(CsvBuffer {
            visible_rows,
            cell_height,
            style,
            top_left_cell_location,
            selection,
            ..
        }) = self;

        let CellLocation { row: row_top, .. } = top_left_cell_location;
        let row_constraints = (0..*visible_rows).map(|_| Constraint::Length(*cell_height));
        let labels = Layout::vertical(row_constraints).spacing(0).split(area);

        for row_label in 0..*visible_rows {
            let row = row_top + row_label;
            let style = if selection.primary.row == row {
                style.label_primary_selection
            } else {
                style.label_normal
            };
            Paragraph::new(CellLocation::row_index_to_id(row))
                .style(style)
                .alignment(Alignment::Center)
                .render(labels[row_label], buf);
        }
    }
}

#[derive(Clone, Debug)]
enum InputState {
    Main(InputModeMain),
    Console(InputModeConsole),
}

impl Default for InputState {
    fn default() -> Self {
        Self::Main(InputModeMain::default())
    }
}

#[derive(Clone, Debug, Default)]
struct InputModeMain {
    mode: MainMode,
    combo: Option<Combo>,
    collect_all: bool,
    input_buffer: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum MainMode {
    #[default]
    Normal,
    Visual,
}
struct StatusWidget<'a>(&'a AppState);

impl<'a> Widget for StatusWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let StatusWidget(state) = self;
        let (mode, buffer_str, combo_str) = match &state.input {
            InputState::Main(InputModeMain {
                mode,
                combo,
                input_buffer,
                ..
            }) => {
                let disp = (*mode == MainMode::Visual)
                    .then(|| ("SEL", Style::default().bg(Color::Blue).fg(Color::Black)));
                (
                    disp,
                    Some(input_buffer),
                    combo.as_ref().map(ToString::to_string),
                )
            }
            InputState::Console(InputModeConsole { mode, .. }) => match mode {
                ConsoleBarMode::Console => (Some(("CON", Style::default())), None, None),
                ConsoleBarMode::CellInput => (
                    Some(("INS", Style::default().bg(Color::Yellow).fg(Color::Black))),
                    None,
                    None,
                ),
            },
        };
        let [mode_area, buffer_area, combo_area, coords_area] = Layout::horizontal([
            Constraint::Length(3),
            Constraint::Length(9),
            Constraint::Length(1),
            Constraint::Length(8),
        ])
        .areas(area);
        if let Some((mode_str, style)) = mode {
            Paragraph::new(mode_str).style(style).render(mode_area, buf);
        }

        if let Some(buffer_str) = buffer_str {
            Paragraph::new(buffer_str.as_str())
                .alignment(Alignment::Right)
                .render(buffer_area, buf);
        }

        if let Some(combo_str) = combo_str {
            Paragraph::new(combo_str.as_str()).render(combo_area, buf);
        }

        if let Some(table) = &state.table {
            Paragraph::new(table.selection.primary.to_string())
                .alignment(Alignment::Right)
                .render(coords_area, buf);
        };
    }
}

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
    /// delimiter used for the FILE
    ///
    /// [default: ,]
    #[arg(short, long)]
    delimiter: Option<char>,
    /// Read csv file from stdin
    #[arg(long, conflicts_with = "file")]
    stdin: bool,
    /// Optional CSV File that will be loaded at start
    #[arg(conflicts_with = "stdin")]
    file: Option<PathBuf>,
}

#[derive(Debug, Copy, Clone, Default)]
struct Selection {
    primary: CellLocation,
    opposite: Option<CellLocation>,
}

#[derive(Debug, Clone)]
enum Yank {
    Single(Option<String>),
    Rectangle {
        cols: usize,
        content: Vec<Option<String>>,
    },
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
            .map(|row| row.as_str().parse::<usize>().map(|u| u.saturating_sub(1)))
            .transpose()
            .map_err(|_| eyre!("Column id too big!"))?;
        let col = caps
            .name("col")
            .map(|col| -> Result<_> {
                let mut result = 0usize;
                for c in col.as_str().chars() {
                    assert!(c.is_ascii_alphabetic());
                    let val = (c.to_ascii_uppercase() as u8 - b'A') as usize + 1;
                    result = result
                        .checked_mul(26)
                        .ok_or_else(|| eyre!("Row id too big!"))?;
                    result = result
                        .checked_add(val)
                        .ok_or_else(|| eyre!("Row id too big!"))?;
                }
                Ok(result - 1)
            })
            .transpose()?;
        if row.is_none() && col.is_none() {
            return Err(eyre!("Emtpy location id!"));
        }
        Ok(Self { sign, row, col })
    }
}
