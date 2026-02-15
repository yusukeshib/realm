use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};
use ratatui::prelude::*;
use ratatui::widgets::{Row, Table, TableState};
use ratatui::{TerminalOptions, Viewport};
use std::io;

use crate::config;
use crate::docker;
use crate::session::{self, SessionSummary};

pub enum TuiAction {
    Resume(String),
    New {
        name: String,
        image: Option<String>,
        command: Option<Vec<String>>,
    },
    Cd(String),
    Quit,
}

#[derive(PartialEq)]
enum Mode {
    Normal,
    DeleteConfirm,
    InputName,
    InputImage,
    InputCommand,
}

struct TextInput {
    text: String,
    cursor: usize,
}

impl TextInput {
    fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
        }
    }

    fn with_text(text: String) -> Self {
        let cursor = text.len();
        Self { text, cursor }
    }

    fn handle_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char(c) => {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let prev = self.text[..self.cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.text.drain(prev..self.cursor);
                    self.cursor = prev;
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.text.len() {
                    let next = self.text[self.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor + i)
                        .unwrap_or(self.text.len());
                    self.text.drain(self.cursor..next);
                }
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor = self.text[..self.cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
            }
            KeyCode::Right => {
                if self.cursor < self.text.len() {
                    self.cursor = self.text[self.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor + i)
                        .unwrap_or(self.text.len());
                }
            }
            _ => {}
        }
    }

    fn to_spans(&self, prefix: &str) -> Vec<Span<'static>> {
        let mut spans = vec![Span::styled(prefix.to_string(), Style::default().bold())];
        let text = &self.text;
        if self.cursor < text.len() {
            let next = text[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(text.len());
            spans.push(Span::raw(text[..self.cursor].to_string()));
            spans.push(Span::styled(
                text[self.cursor..next].to_string(),
                Style::default().reversed(),
            ));
            spans.push(Span::raw(text[next..].to_string()));
        } else {
            spans.push(Span::raw(text.clone()));
            spans.push(Span::styled(" ", Style::default().reversed()));
        }
        spans
    }
}

struct TermGuard;

impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

fn clear_viewport(
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
    height: u16,
) -> Result<()> {
    terminal.clear()?;
    execute!(
        io::stderr(),
        cursor::MoveUp(height),
        terminal::Clear(terminal::ClearType::FromCursorDown)
    )?;
    Ok(())
}

pub fn session_manager<F>(sessions: &[SessionSummary], delete_fn: F) -> Result<TuiAction>
where
    F: Fn(&str) -> Result<()>,
{
    let mut items: Vec<SessionSummary> = sessions.to_vec();
    // +1 for "new session" row, +1 for header, +1 for footer
    let viewport_height = (items.len() as u16) + 3;

    terminal::enable_raw_mode()?;
    let _guard = TermGuard;

    let options = TerminalOptions {
        viewport: Viewport::Inline(viewport_height),
    };
    let mut terminal = Terminal::with_options(CrosstermBackend::new(io::stderr()), options)?;
    let mut state = TableState::default();
    state.select(Some(0));
    // Row 0 = "new session", rows 1.. = actual sessions
    let new_row_idx = 0;

    let mut mode = Mode::Normal;
    let mut input = TextInput::new();
    let mut footer_msg = String::new();
    let mut new_name = String::new();
    let mut new_image: Option<String> = None;

    loop {
        terminal.draw(|f| {
            let area = f.area();
            // Reserve last row for footer
            let table_area = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: area.height.saturating_sub(1),
            };
            let footer_area = Rect {
                x: area.x,
                y: area.y + area.height.saturating_sub(1),
                width: area.width,
                height: 1,
            };

            // Table
            {
                let header = Row::new(["NAME", "STATUS", "PROJECT", "IMAGE", "CMD", "CREATED"])
                    .style(Style::default().dim());

                let total_rows = 1 + items.len(); // "new session" + actual sessions
                let mut rows: Vec<Row> = Vec::with_capacity(total_rows);

                // First row: "+ new session"
                rows.push(Row::new(["New box...", "", "", "", "", ""]));

                // Session rows
                for (i, s) in items.iter().enumerate() {
                    let status = if s.running { "running" } else { "" };
                    let row = Row::new([
                        s.name.as_str(),
                        status,
                        s.project_dir.as_str(),
                        s.image.as_str(),
                        s.command.as_str(),
                        s.created_at.as_str(),
                    ]);
                    let row_idx = i + 1; // offset by "new session" row
                    if mode == Mode::DeleteConfirm && state.selected() == Some(row_idx) {
                        rows.push(row.style(Style::default().fg(Color::Red)));
                    } else {
                        rows.push(row);
                    }
                }

                let widths = [
                    Constraint::Min(15),
                    Constraint::Min(10),
                    Constraint::Min(30),
                    Constraint::Min(20),
                    Constraint::Min(15),
                    Constraint::Min(22),
                ];

                let table = Table::new(rows, widths)
                    .header(header)
                    .highlight_symbol("> ")
                    .row_highlight_style(Style::default().bold());

                f.render_stateful_widget(table, table_area, &mut state);
            }

            // Footer
            let on_new_row = state.selected() == Some(new_row_idx);
            let footer_line: Line = match &mode {
                Mode::Normal => {
                    if !footer_msg.is_empty() {
                        Line::from(Span::styled(
                            footer_msg.as_str(),
                            Style::default().fg(Color::Red),
                        ))
                    } else if on_new_row || items.is_empty() {
                        Line::from("[Enter] New  [q] Quit").style(Style::default().dim())
                    } else {
                        Line::from("[Enter] Resume  [c] Cd  [d] Delete  [q] Quit")
                            .style(Style::default().dim())
                    }
                }
                Mode::DeleteConfirm => {
                    let name = state
                        .selected()
                        .and_then(|i| items.get(i.saturating_sub(1)))
                        .map(|s| s.name.as_str())
                        .unwrap_or("");
                    Line::from(format!("Delete '{}'? [y/n]", name)).style(Style::default().dim())
                }
                Mode::InputName => Line::from(input.to_spans("Session name: ")),
                Mode::InputImage => Line::from(input.to_spans("Image: ")),
                Mode::InputCommand => Line::from(input.to_spans("Command (optional): ")),
            };
            f.render_widget(footer_line, footer_area);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            // Ctrl+C in any mode → quit
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                clear_viewport(&mut terminal, viewport_height)?;
                return Ok(TuiAction::Quit);
            }

            match mode {
                Mode::Normal => {
                    footer_msg.clear();
                    let total_rows = 1 + items.len(); // "new session" + sessions
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            let i = state.selected().unwrap_or(0);
                            let next = if i == 0 { total_rows - 1 } else { i - 1 };
                            state.select(Some(next));
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let i = state.selected().unwrap_or(0);
                            let next = if i >= total_rows - 1 { 0 } else { i + 1 };
                            state.select(Some(next));
                        }
                        KeyCode::Enter => {
                            if let Some(i) = state.selected() {
                                if i == new_row_idx {
                                    input = TextInput::new();
                                    mode = Mode::InputName;
                                } else {
                                    let name = items[i - 1].name.clone();
                                    clear_viewport(&mut terminal, viewport_height)?;
                                    return Ok(TuiAction::Resume(name));
                                }
                            }
                        }
                        KeyCode::Char('c') => {
                            if let Some(i) = state.selected() {
                                if i != new_row_idx {
                                    let name = items[i - 1].name.clone();
                                    clear_viewport(&mut terminal, viewport_height)?;
                                    return Ok(TuiAction::Cd(name));
                                }
                            }
                        }
                        KeyCode::Char('d') => {
                            if let Some(i) = state.selected() {
                                if i != new_row_idx {
                                    mode = Mode::DeleteConfirm;
                                }
                            }
                        }
                        KeyCode::Esc | KeyCode::Char('q') => {
                            clear_viewport(&mut terminal, viewport_height)?;
                            return Ok(TuiAction::Quit);
                        }
                        _ => {}
                    }
                }
                Mode::DeleteConfirm => match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        if let Some(i) = state.selected() {
                            let item_idx = i - 1; // offset for "new session" row
                            let name = items[item_idx].name.clone();
                            if let Err(e) = delete_fn(&name) {
                                footer_msg = format!("Delete failed: {}", e);
                            }
                            // Refresh list
                            if let Ok(mut refreshed) = session::list() {
                                if let Ok(running) =
                                    std::panic::catch_unwind(docker::running_sessions)
                                {
                                    for s in &mut refreshed {
                                        s.running = running.contains(&s.name);
                                    }
                                }
                                items = refreshed;
                            }
                            let total_rows = 1 + items.len();
                            if i >= total_rows {
                                state.select(Some(total_rows - 1));
                            }
                        }
                        mode = Mode::Normal;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        mode = Mode::Normal;
                    }
                    _ => {}
                },
                Mode::InputName => match key.code {
                    KeyCode::Enter => {
                        let name = input.text.trim().to_string();
                        if let Err(e) = session::validate_name(&name) {
                            footer_msg = e.to_string();
                            mode = Mode::Normal;
                            input = TextInput::new();
                        } else if session::session_exists(&name).unwrap_or(false) {
                            footer_msg = format!("Session '{}' already exists.", name);
                            mode = Mode::Normal;
                            input = TextInput::new();
                        } else {
                            new_name = name;
                            let default_image = std::env::var("BOX_DEFAULT_IMAGE")
                                .unwrap_or_else(|_| config::DEFAULT_IMAGE.to_string());
                            input = TextInput::with_text(default_image);
                            mode = Mode::InputImage;
                        }
                    }
                    KeyCode::Esc => {
                        mode = Mode::Normal;
                    }
                    _ => {
                        input.handle_key(key.code);
                    }
                },
                Mode::InputImage => match key.code {
                    KeyCode::Enter => {
                        let image_text = input.text.trim().to_string();
                        new_image = if image_text.is_empty() {
                            None
                        } else {
                            Some(image_text)
                        };
                        let default_cmd = std::env::var("BOX_DEFAULT_CMD").unwrap_or_default();
                        input = TextInput::with_text(default_cmd);
                        mode = Mode::InputCommand;
                    }
                    KeyCode::Esc => {
                        mode = Mode::Normal;
                    }
                    _ => {
                        input.handle_key(key.code);
                    }
                },
                Mode::InputCommand => match key.code {
                    KeyCode::Enter => {
                        let cmd_text = input.text.trim().to_string();
                        let command = if cmd_text.is_empty() {
                            Some(vec![])
                        } else {
                            match shell_words::split(&cmd_text) {
                                Ok(args) => Some(args),
                                Err(e) => {
                                    footer_msg = format!("Invalid command: {e}");
                                    mode = Mode::Normal;
                                    input = TextInput::new();
                                    continue;
                                }
                            }
                        };
                        clear_viewport(&mut terminal, viewport_height)?;
                        return Ok(TuiAction::New {
                            name: new_name,
                            image: new_image,
                            command,
                        });
                    }
                    KeyCode::Esc => {
                        mode = Mode::Normal;
                    }
                    _ => {
                        input.handle_key(key.code);
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_input_insert() {
        let mut input = TextInput::new();
        input.handle_key(KeyCode::Char('a'));
        input.handle_key(KeyCode::Char('b'));
        input.handle_key(KeyCode::Char('c'));
        assert_eq!(input.text, "abc");
        assert_eq!(input.cursor, 3);
    }

    #[test]
    fn test_text_input_backspace() {
        let mut input = TextInput::new();
        input.handle_key(KeyCode::Char('a'));
        input.handle_key(KeyCode::Char('b'));
        input.handle_key(KeyCode::Backspace);
        assert_eq!(input.text, "a");
        assert_eq!(input.cursor, 1);
    }

    #[test]
    fn test_text_input_backspace_at_start() {
        let mut input = TextInput::new();
        input.handle_key(KeyCode::Backspace);
        assert_eq!(input.text, "");
        assert_eq!(input.cursor, 0);
    }

    #[test]
    fn test_text_input_delete() {
        let mut input = TextInput::new();
        input.handle_key(KeyCode::Char('a'));
        input.handle_key(KeyCode::Char('b'));
        input.handle_key(KeyCode::Left);
        input.handle_key(KeyCode::Delete);
        assert_eq!(input.text, "a");
        assert_eq!(input.cursor, 1);
    }

    #[test]
    fn test_text_input_delete_at_end() {
        let mut input = TextInput::new();
        input.handle_key(KeyCode::Char('a'));
        input.handle_key(KeyCode::Delete);
        assert_eq!(input.text, "a");
        assert_eq!(input.cursor, 1);
    }

    #[test]
    fn test_text_input_cursor_movement() {
        let mut input = TextInput::new();
        input.handle_key(KeyCode::Char('a'));
        input.handle_key(KeyCode::Char('b'));
        input.handle_key(KeyCode::Char('c'));
        input.handle_key(KeyCode::Left);
        input.handle_key(KeyCode::Left);
        assert_eq!(input.cursor, 1);
        input.handle_key(KeyCode::Right);
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn test_text_input_left_at_start() {
        let mut input = TextInput::new();
        input.handle_key(KeyCode::Char('a'));
        input.handle_key(KeyCode::Left);
        input.handle_key(KeyCode::Left); // should not go below 0
        assert_eq!(input.cursor, 0);
    }

    #[test]
    fn test_text_input_right_at_end() {
        let mut input = TextInput::new();
        input.handle_key(KeyCode::Char('a'));
        input.handle_key(KeyCode::Right); // should not go past len
        assert_eq!(input.cursor, 1);
    }

    #[test]
    fn test_text_input_insert_at_cursor() {
        let mut input = TextInput::new();
        input.handle_key(KeyCode::Char('a'));
        input.handle_key(KeyCode::Char('c'));
        input.handle_key(KeyCode::Left);
        input.handle_key(KeyCode::Char('b'));
        assert_eq!(input.text, "abc");
        assert_eq!(input.cursor, 2);
    }
}
