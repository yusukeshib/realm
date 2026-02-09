use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal;
use ratatui::prelude::*;
use ratatui::widgets::{Row, Table, TableState};
use ratatui::{TerminalOptions, Viewport};
use std::io;

use crate::session::SessionSummary;

struct TermGuard;

impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

pub fn select_sessions_to_delete(sessions: &[SessionSummary]) -> Result<Vec<usize>> {
    let height = (sessions.len() as u16) + 1;

    terminal::enable_raw_mode()?;
    let _guard = TermGuard;

    let options = TerminalOptions {
        viewport: Viewport::Inline(height),
    };
    let mut terminal = Terminal::with_options(CrosstermBackend::new(io::stderr()), options)?;
    let mut state = TableState::default();
    state.select(Some(0));
    let mut checked = vec![false; sessions.len()];

    loop {
        terminal.draw(|f| {
            let header = Row::new(["", "NAME", "STATUS", "PROJECT", "IMAGE", "CREATED"])
                .style(Style::default().dim());

            let rows: Vec<Row> = sessions
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let check = if checked[i] { "[x]" } else { "[ ]" };
                    let status = if s.running { "running" } else { "" };
                    Row::new([
                        check,
                        s.name.as_str(),
                        status,
                        s.project_dir.as_str(),
                        s.image.as_str(),
                        s.created_at.as_str(),
                    ])
                })
                .collect();

            let widths = [
                Constraint::Max(3),
                Constraint::Min(15),
                Constraint::Min(10),
                Constraint::Min(30),
                Constraint::Min(20),
                Constraint::Min(22),
            ];

            let table = Table::new(rows, widths)
                .header(header)
                .highlight_symbol("> ")
                .row_highlight_style(Style::default().bold());

            f.render_stateful_widget(table, f.area(), &mut state);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    let i = state.selected().unwrap_or(0);
                    let next = if i == 0 { sessions.len() - 1 } else { i - 1 };
                    state.select(Some(next));
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let i = state.selected().unwrap_or(0);
                    let next = if i >= sessions.len() - 1 { 0 } else { i + 1 };
                    state.select(Some(next));
                }
                KeyCode::Char(' ') => {
                    if let Some(i) = state.selected() {
                        checked[i] = !checked[i];
                    }
                }
                KeyCode::Enter => {
                    return Ok(checked
                        .iter()
                        .enumerate()
                        .filter(|(_, &c)| c)
                        .map(|(i, _)| i)
                        .collect());
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    return Ok(vec![]);
                }
                _ => {}
            }
        }
    }
}

pub fn select_session(sessions: &[SessionSummary]) -> Result<Option<usize>> {
    // header + rows + 1 blank line so the prompt stays beneath
    let height = (sessions.len() as u16) + 1;

    terminal::enable_raw_mode()?;
    let _guard = TermGuard;

    let options = TerminalOptions {
        viewport: Viewport::Inline(height),
    };
    let mut terminal = Terminal::with_options(CrosstermBackend::new(io::stderr()), options)?;
    let mut state = TableState::default();
    state.select(Some(0));

    loop {
        terminal.draw(|f| {
            let header = Row::new(["NAME", "STATUS", "PROJECT", "IMAGE", "CREATED"])
                .style(Style::default().dim());

            let rows: Vec<Row> = sessions
                .iter()
                .map(|s| {
                    let status = if s.running { "running" } else { "" };
                    Row::new([
                        s.name.as_str(),
                        status,
                        s.project_dir.as_str(),
                        s.image.as_str(),
                        s.created_at.as_str(),
                    ])
                })
                .collect();

            let widths = [
                Constraint::Min(15),
                Constraint::Min(10),
                Constraint::Min(30),
                Constraint::Min(20),
                Constraint::Min(22),
            ];

            let table = Table::new(rows, widths)
                .header(header)
                .highlight_symbol("> ")
                .row_highlight_style(Style::default().bold());

            f.render_stateful_widget(table, f.area(), &mut state);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    let i = state.selected().unwrap_or(0);
                    let next = if i == 0 { sessions.len() - 1 } else { i - 1 };
                    state.select(Some(next));
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let i = state.selected().unwrap_or(0);
                    let next = if i >= sessions.len() - 1 { 0 } else { i + 1 };
                    state.select(Some(next));
                }
                KeyCode::Enter => {
                    return Ok(state.selected());
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    return Ok(None);
                }
                _ => {}
            }
        }
    }
}
