use anyhow::{Context, Result};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::{execute, terminal};
use nix::libc;
use nix::pty::openpty;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use std::io::{self, Read};
use std::os::fd::BorrowedFd;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::process::Child;
use std::sync::mpsc;
use std::time::Duration;

use crate::docker;

pub enum OverlayResult {
    Detached,
    Exited(i32),
    Stopped,
}

/// Run a Docker session with a TUI overlay that shows a title bar.
///
/// `spawn_docker` receives the slave PTY fd and must return a `Child`.
pub fn run_with_overlay(
    session_name: &str,
    title_color: Option<&str>,
    spawn_docker: impl FnOnce(RawFd) -> Result<Child>,
) -> Result<OverlayResult> {
    let color = parse_color(title_color);

    // Get terminal size
    let (term_cols, term_rows) = terminal::size().context("Failed to get terminal size")?;
    if term_rows < 3 || term_cols < 20 {
        anyhow::bail!("Terminal too small (need at least 20x3)");
    }

    // Content area is term_rows - 1 (row 0 is title bar)
    let content_rows = term_rows - 1;

    // Open PTY with content area size
    let pty = openpty(
        Some(&nix::pty::Winsize {
            ws_row: content_rows,
            ws_col: term_cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }),
        None,
    )
    .context("Failed to open PTY")?;

    let master_fd = pty.master.as_raw_fd();
    let slave_fd = pty.slave.as_raw_fd();

    // Spawn docker with the slave fd
    let mut child = spawn_docker(slave_fd)?;

    // Close slave fd in parent process - Docker child owns it now
    drop(pty.slave);

    // Set up vt100 parser
    let mut parser = vt100::Parser::new(content_rows, term_cols, 0);

    // Reader thread: reads PTY master output, sends to main thread
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let master_read_fd = unsafe { libc::dup(master_fd) };
    if master_read_fd < 0 {
        anyhow::bail!("Failed to dup master fd");
    }
    let reader_thread = std::thread::spawn(move || {
        let mut file = unsafe { std::fs::File::from_raw_fd(master_read_fd) };
        let mut buf = [0u8; 4096];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });

    // Enter alternate screen + raw mode + mouse capture
    let mut stdout = io::stdout();
    execute!(stdout, terminal::EnterAlternateScreen, EnableMouseCapture)?;
    terminal::enable_raw_mode()?;

    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Detach state machine: Ctrl+P then Q
    let mut ctrl_p_pressed = false;

    // Track mouse mode from inner terminal
    let mut mouse_mode = false;

    let result = run_event_loop(
        &mut terminal,
        &mut child,
        &mut parser,
        &rx,
        master_fd,
        session_name,
        color,
        &mut ctrl_p_pressed,
        &mut mouse_mode,
    );

    // Clean up reader thread
    drop(pty.master);
    let _ = reader_thread.join();

    result
}

#[allow(clippy::too_many_arguments)]
fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    child: &mut Child,
    parser: &mut vt100::Parser,
    rx: &mpsc::Receiver<Vec<u8>>,
    master_fd: RawFd,
    session_name: &str,
    title_color: Color,
    ctrl_p_pressed: &mut bool,
    mouse_mode: &mut bool,
) -> Result<OverlayResult> {
    loop {
        // Drain PTY output
        let mut got_output = false;
        loop {
            match rx.try_recv() {
                Ok(data) => {
                    // Check for mouse mode escape sequences in output
                    detect_mouse_mode(&data, mouse_mode);
                    parser.process(&data);
                    got_output = true;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }

        // Check if child exited
        match child.try_wait() {
            Ok(Some(status)) => {
                // Drain remaining output
                while let Ok(data) = rx.try_recv() {
                    parser.process(&data);
                }
                // Final render
                render(terminal, parser, session_name, title_color)?;
                return Ok(OverlayResult::Exited(status.code().unwrap_or(1)));
            }
            Ok(None) => {}
            Err(_) => return Ok(OverlayResult::Exited(1)),
        }

        // Render
        if got_output || !event::poll(Duration::ZERO)? {
            render(terminal, parser, session_name, title_color)?;
        }

        // Poll for events with timeout for ~60fps
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
                        continue;
                    }

                    // Detach: Ctrl+P, Q
                    if *ctrl_p_pressed {
                        *ctrl_p_pressed = false;
                        if key.code == KeyCode::Char('q') || key.code == KeyCode::Char('Q') {
                            return Ok(OverlayResult::Detached);
                        }
                        // Not Q - send the buffered Ctrl+P then this key
                        write_to_pty(master_fd, &[0x10]); // Ctrl+P
                        let bytes = key_to_bytes(key, parser.screen());
                        write_to_pty(master_fd, &bytes);
                        continue;
                    }

                    if key.code == KeyCode::Char('p')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        *ctrl_p_pressed = true;
                        continue;
                    }

                    let bytes = key_to_bytes(key, parser.screen());
                    write_to_pty(master_fd, &bytes);
                }
                Event::Mouse(mouse) => {
                    let (_, term_rows) = terminal::size().unwrap_or((80, 24));
                    handle_mouse(
                        mouse,
                        master_fd,
                        session_name,
                        term_rows,
                        child,
                        *mouse_mode,
                    )?;

                    // Check if we got a stop/detach action from title bar
                    if let Some(action) = check_title_bar_click(mouse, term_rows, session_name) {
                        match action {
                            TitleBarAction::Detach => return Ok(OverlayResult::Detached),
                            TitleBarAction::Stop => {
                                docker::stop_container(session_name);
                                let _ = child.wait();
                                return Ok(OverlayResult::Stopped);
                            }
                        }
                    }
                }
                Event::Resize(cols, rows) => {
                    if rows > 1 {
                        let content_rows = rows - 1;
                        parser.set_size(content_rows, cols);
                        set_pty_size(master_fd, content_rows, cols);
                    }
                }
                _ => {}
            }
        }
    }
}

fn render(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    parser: &vt100::Parser,
    session_name: &str,
    title_color: Color,
) -> Result<()> {
    let screen = parser.screen();
    terminal.draw(|f| {
        let area = f.area();
        if area.height < 2 {
            return;
        }

        let title_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        let content_area = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height - 1,
        };

        // Render title bar
        render_title_bar(f, title_area, session_name, title_color);

        // Render terminal content
        render_content(f, content_area, screen);

        // Position cursor
        if !screen.hide_cursor() {
            let cursor = screen.cursor_position();
            let cx = content_area.x + cursor.1;
            let cy = content_area.y + cursor.0;
            if cx < content_area.x + content_area.width && cy < content_area.y + content_area.height
            {
                f.set_cursor_position((cx, cy));
            }
        }
    })?;
    Ok(())
}

fn render_title_bar(f: &mut Frame, area: Rect, session_name: &str, title_color: Color) {
    let width = area.width as usize;
    if width < 10 {
        return;
    }

    let detach_btn = "[_]";
    let stop_btn = "[x]";
    let buttons = format!(" {} {} ", detach_btn, stop_btn);
    let buttons_len = buttons.len();
    let name_max = width.saturating_sub(buttons_len + 2);
    let display_name = if session_name.len() > name_max {
        &session_name[..name_max]
    } else {
        session_name
    };
    let padding = width.saturating_sub(display_name.len() + 1 + buttons_len);

    let spans = vec![
        Span::styled(
            format!(" {}", display_name),
            Style::default().fg(title_color).bg(Color::DarkGray).bold(),
        ),
        Span::styled(" ".repeat(padding), Style::default().bg(Color::DarkGray)),
        Span::styled(
            buttons,
            Style::default().fg(Color::White).bg(Color::DarkGray),
        ),
    ];

    let line = Line::from(spans);
    f.render_widget(Paragraph::new(line), area);
}

fn render_content(f: &mut Frame, area: Rect, screen: &vt100::Screen) {
    let rows = area.height;
    let cols = area.width;

    let buf = f.buffer_mut();

    for row in 0..rows {
        for col in 0..cols {
            let cell = screen.cell(row, col);
            let x = area.x + col;
            let y = area.y + row;

            if x >= buf.area.x + buf.area.width || y >= buf.area.y + buf.area.height {
                continue;
            }

            if let Some(cell) = cell {
                if cell.is_wide_continuation() {
                    continue;
                }

                let buf_cell = &mut buf[(x, y)];
                let ch = cell.contents();
                if ch.is_empty() {
                    buf_cell.set_char(' ');
                } else {
                    // Set the symbol (may be multi-char for wide chars)
                    buf_cell.set_symbol(&ch);
                }

                let fg = convert_color(cell.fgcolor());
                let bg = convert_color(cell.bgcolor());
                let mut style = Style::default().fg(fg).bg(bg);

                if cell.bold() {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if cell.italic() {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                if cell.underline() {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                if cell.inverse() {
                    style = style.add_modifier(Modifier::REVERSED);
                }

                buf_cell.set_style(style);

                if cell.is_wide() {
                    // Clear the continuation cell
                    let next_x = x + 1;
                    if next_x < area.x + area.width {
                        let next_cell = &mut buf[(next_x, y)];
                        next_cell.set_symbol("");
                        next_cell.set_style(style);
                    }
                }
            }
        }
    }
}

fn convert_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn parse_color(color: Option<&str>) -> Color {
    match color {
        None => Color::White,
        Some(s) => {
            // Try hex color
            if let Some(hex) = s.strip_prefix('#') {
                if hex.len() == 6 {
                    if let (Ok(r), Ok(g), Ok(b)) = (
                        u8::from_str_radix(&hex[0..2], 16),
                        u8::from_str_radix(&hex[2..4], 16),
                        u8::from_str_radix(&hex[4..6], 16),
                    ) {
                        return Color::Rgb(r, g, b);
                    }
                }
            }
            // Named colors
            match s.to_lowercase().as_str() {
                "black" => Color::Black,
                "red" => Color::Red,
                "green" => Color::Green,
                "yellow" => Color::Yellow,
                "blue" => Color::Blue,
                "magenta" => Color::Magenta,
                "cyan" => Color::Cyan,
                "white" => Color::White,
                "gray" | "grey" => Color::Gray,
                "darkgray" | "darkgrey" => Color::DarkGray,
                "lightred" => Color::LightRed,
                "lightgreen" => Color::LightGreen,
                "lightyellow" => Color::LightYellow,
                "lightblue" => Color::LightBlue,
                "lightmagenta" => Color::LightMagenta,
                "lightcyan" => Color::LightCyan,
                _ => Color::White,
            }
        }
    }
}

fn key_to_bytes(key: KeyEvent, screen: &vt100::Screen) -> Vec<u8> {
    let app_cursor = screen.application_cursor();
    let _app_keypad = screen.application_keypad();

    // Handle Ctrl+<key> combinations
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(c) = key.code {
            let byte = match c {
                'a'..='z' => c as u8 - b'a' + 1,
                'A'..='Z' => c as u8 - b'A' + 1,
                '@' => 0,
                '[' => 27,
                '\\' => 28,
                ']' => 29,
                '^' => 30,
                '_' => 31,
                _ => return vec![],
            };
            return vec![byte];
        }
    }

    // Handle Alt+<key>
    if key.modifiers.contains(KeyModifiers::ALT) {
        if let KeyCode::Char(c) = key.code {
            let mut bytes = vec![0x1b]; // ESC
            let mut char_buf = [0u8; 4];
            let encoded = c.encode_utf8(&mut char_buf);
            bytes.extend_from_slice(encoded.as_bytes());
            return bytes;
        }
    }

    match key.code {
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            let encoded = c.encode_utf8(&mut buf);
            encoded.as_bytes().to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => vec![0x1b, b'[', b'Z'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => {
            if app_cursor {
                vec![0x1b, b'O', b'A']
            } else {
                vec![0x1b, b'[', b'A']
            }
        }
        KeyCode::Down => {
            if app_cursor {
                vec![0x1b, b'O', b'B']
            } else {
                vec![0x1b, b'[', b'B']
            }
        }
        KeyCode::Right => {
            if app_cursor {
                vec![0x1b, b'O', b'C']
            } else {
                vec![0x1b, b'[', b'C']
            }
        }
        KeyCode::Left => {
            if app_cursor {
                vec![0x1b, b'O', b'D']
            } else {
                vec![0x1b, b'[', b'D']
            }
        }
        KeyCode::Home => {
            if app_cursor {
                vec![0x1b, b'O', b'H']
            } else {
                vec![0x1b, b'[', b'H']
            }
        }
        KeyCode::End => {
            if app_cursor {
                vec![0x1b, b'O', b'F']
            } else {
                vec![0x1b, b'[', b'F']
            }
        }
        KeyCode::Insert => vec![0x1b, b'[', b'2', b'~'],
        KeyCode::Delete => vec![0x1b, b'[', b'3', b'~'],
        KeyCode::PageUp => vec![0x1b, b'[', b'5', b'~'],
        KeyCode::PageDown => vec![0x1b, b'[', b'6', b'~'],
        KeyCode::F(n) => f_key_bytes(n),
        _ => vec![],
    }
}

fn f_key_bytes(n: u8) -> Vec<u8> {
    match n {
        1 => vec![0x1b, b'O', b'P'],
        2 => vec![0x1b, b'O', b'Q'],
        3 => vec![0x1b, b'O', b'R'],
        4 => vec![0x1b, b'O', b'S'],
        5 => vec![0x1b, b'[', b'1', b'5', b'~'],
        6 => vec![0x1b, b'[', b'1', b'7', b'~'],
        7 => vec![0x1b, b'[', b'1', b'8', b'~'],
        8 => vec![0x1b, b'[', b'1', b'9', b'~'],
        9 => vec![0x1b, b'[', b'2', b'0', b'~'],
        10 => vec![0x1b, b'[', b'2', b'1', b'~'],
        11 => vec![0x1b, b'[', b'2', b'3', b'~'],
        12 => vec![0x1b, b'[', b'2', b'4', b'~'],
        _ => vec![],
    }
}

fn write_to_pty(master_fd: RawFd, data: &[u8]) {
    if data.is_empty() {
        return;
    }
    let fd = unsafe { BorrowedFd::borrow_raw(master_fd) };
    let _ = nix::unistd::write(fd, data);
}

fn set_pty_size(master_fd: RawFd, rows: u16, cols: u16) {
    let ws = nix::pty::Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        libc::ioctl(master_fd, libc::TIOCSWINSZ, &ws);
    }
}

/// Detect whether the inner application is requesting mouse tracking.
fn detect_mouse_mode(data: &[u8], mouse_mode: &mut bool) {
    // Look for common mouse enable/disable sequences
    // Enable: \e[?1000h, \e[?1002h, \e[?1003h, \e[?1006h
    // Disable: \e[?1000l, \e[?1002l, \e[?1003l, \e[?1006l
    let s = match std::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => return,
    };
    // Simple detection - check for the last relevant sequence
    for window in ["\x1b[?1000h", "\x1b[?1002h", "\x1b[?1003h", "\x1b[?1006h"] {
        if s.contains(window) {
            *mouse_mode = true;
        }
    }
    for window in ["\x1b[?1000l", "\x1b[?1002l", "\x1b[?1003l", "\x1b[?1006l"] {
        if s.contains(window) {
            *mouse_mode = false;
        }
    }
}

enum TitleBarAction {
    Detach,
    Stop,
}

fn check_title_bar_click(
    mouse: MouseEvent,
    _term_rows: u16,
    _session_name: &str,
) -> Option<TitleBarAction> {
    // Only handle clicks on row 0 (title bar)
    if mouse.row != 0 {
        return None;
    }
    if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
        return None;
    }

    let (term_cols, _) = terminal::size().unwrap_or((80, 24));
    let width = term_cols as usize;

    // Button layout: " session_name <padding> [_] [x] "
    let detach_btn = "[_]";
    let stop_btn = "[x]";
    let buttons_str = format!(" {} {} ", detach_btn, stop_btn);
    let buttons_len = buttons_str.len();

    let col = mouse.column as usize;

    // Buttons are right-aligned
    let buttons_start = width.saturating_sub(buttons_len);

    // [_] starts at buttons_start + 1, length 3
    let detach_start = buttons_start + 1;
    let detach_end = detach_start + 3;

    // [x] starts at detach_end + 1, length 3
    let stop_start = detach_end + 1;
    let stop_end = stop_start + 3;

    if col >= detach_start && col < detach_end {
        Some(TitleBarAction::Detach)
    } else if col >= stop_start && col < stop_end {
        Some(TitleBarAction::Stop)
    } else {
        None
    }
}

fn handle_mouse(
    mouse: MouseEvent,
    master_fd: RawFd,
    _session_name: &str,
    _term_rows: u16,
    _child: &mut Child,
    mouse_mode: bool,
) -> Result<()> {
    // Title bar clicks are handled separately in the event loop
    if mouse.row == 0 {
        return Ok(());
    }

    // Forward mouse events to inner terminal if mouse mode is active
    if !mouse_mode {
        return Ok(());
    }

    // Translate row: subtract 1 for title bar
    let inner_row = mouse.row.saturating_sub(1);
    let col = mouse.column;

    // Use SGR (1006) mouse encoding: \e[<Cb;Cx;CyM or \e[<Cb;Cx;Cym
    let (button, suffix) = match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => (0, 'M'),
        MouseEventKind::Down(MouseButton::Right) => (2, 'M'),
        MouseEventKind::Down(MouseButton::Middle) => (1, 'M'),
        MouseEventKind::Up(MouseButton::Left) => (0, 'm'),
        MouseEventKind::Up(MouseButton::Right) => (2, 'm'),
        MouseEventKind::Up(MouseButton::Middle) => (1, 'm'),
        MouseEventKind::Drag(MouseButton::Left) => (32, 'M'),
        MouseEventKind::Drag(MouseButton::Right) => (34, 'M'),
        MouseEventKind::Drag(MouseButton::Middle) => (33, 'M'),
        MouseEventKind::Moved => (35, 'M'),
        MouseEventKind::ScrollUp => (64, 'M'),
        MouseEventKind::ScrollDown => (65, 'M'),
        MouseEventKind::ScrollLeft => (66, 'M'),
        MouseEventKind::ScrollRight => (67, 'M'),
    };

    // SGR encoding: \e[<button;col+1;row+1M/m
    let seq = format!("\x1b[<{};{};{}{}", button, col + 1, inner_row + 1, suffix);
    write_to_pty(master_fd, seq.as_bytes());

    Ok(())
}

/// RAII guard for terminal cleanup
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableMouseCapture,
            terminal::LeaveAlternateScreen
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_color_none() {
        assert_eq!(parse_color(None), Color::White);
    }

    #[test]
    fn test_parse_color_named() {
        assert_eq!(parse_color(Some("blue")), Color::Blue);
        assert_eq!(parse_color(Some("Blue")), Color::Blue);
        assert_eq!(parse_color(Some("RED")), Color::Red);
        assert_eq!(parse_color(Some("green")), Color::Green);
        assert_eq!(parse_color(Some("yellow")), Color::Yellow);
        assert_eq!(parse_color(Some("cyan")), Color::Cyan);
        assert_eq!(parse_color(Some("magenta")), Color::Magenta);
        assert_eq!(parse_color(Some("white")), Color::White);
        assert_eq!(parse_color(Some("black")), Color::Black);
        assert_eq!(parse_color(Some("gray")), Color::Gray);
        assert_eq!(parse_color(Some("grey")), Color::Gray);
    }

    #[test]
    fn test_parse_color_hex() {
        assert_eq!(parse_color(Some("#ff0000")), Color::Rgb(255, 0, 0));
        assert_eq!(parse_color(Some("#00ff00")), Color::Rgb(0, 255, 0));
        assert_eq!(parse_color(Some("#0000ff")), Color::Rgb(0, 0, 255));
        assert_eq!(parse_color(Some("#abcdef")), Color::Rgb(171, 205, 239));
    }

    #[test]
    fn test_parse_color_invalid_hex() {
        // Too short
        assert_eq!(parse_color(Some("#fff")), Color::White);
        // Invalid chars
        assert_eq!(parse_color(Some("#gggggg")), Color::White);
    }

    #[test]
    fn test_parse_color_unknown() {
        assert_eq!(parse_color(Some("unknown")), Color::White);
    }

    #[test]
    fn test_key_to_bytes_char() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        let parser = vt100::Parser::new(24, 80, 0);
        assert_eq!(key_to_bytes(key, parser.screen()), vec![b'a']);
    }

    #[test]
    fn test_key_to_bytes_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let parser = vt100::Parser::new(24, 80, 0);
        assert_eq!(key_to_bytes(key, parser.screen()), vec![b'\r']);
    }

    #[test]
    fn test_key_to_bytes_ctrl_c() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let parser = vt100::Parser::new(24, 80, 0);
        assert_eq!(key_to_bytes(key, parser.screen()), vec![3]);
    }

    #[test]
    fn test_key_to_bytes_arrow_keys() {
        let parser = vt100::Parser::new(24, 80, 0);
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(key_to_bytes(up, parser.screen()), vec![0x1b, b'[', b'A']);
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(key_to_bytes(down, parser.screen()), vec![0x1b, b'[', b'B']);
    }

    #[test]
    fn test_key_to_bytes_esc() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let parser = vt100::Parser::new(24, 80, 0);
        assert_eq!(key_to_bytes(key, parser.screen()), vec![0x1b]);
    }

    #[test]
    fn test_key_to_bytes_backspace() {
        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        let parser = vt100::Parser::new(24, 80, 0);
        assert_eq!(key_to_bytes(key, parser.screen()), vec![0x7f]);
    }

    #[test]
    fn test_key_to_bytes_f_keys() {
        let parser = vt100::Parser::new(24, 80, 0);
        let f1 = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
        assert_eq!(key_to_bytes(f1, parser.screen()), vec![0x1b, b'O', b'P']);
        let f5 = KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE);
        assert_eq!(
            key_to_bytes(f5, parser.screen()),
            vec![0x1b, b'[', b'1', b'5', b'~']
        );
    }

    #[test]
    fn test_key_to_bytes_alt_char() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT);
        let parser = vt100::Parser::new(24, 80, 0);
        assert_eq!(key_to_bytes(key, parser.screen()), vec![0x1b, b'x']);
    }

    #[test]
    fn test_detect_mouse_mode_enable() {
        let mut mode = false;
        detect_mouse_mode(b"\x1b[?1000h", &mut mode);
        assert!(mode);
    }

    #[test]
    fn test_detect_mouse_mode_disable() {
        let mut mode = true;
        detect_mouse_mode(b"\x1b[?1000l", &mut mode);
        assert!(!mode);
    }

    #[test]
    fn test_detect_mouse_mode_sgr_enable() {
        let mut mode = false;
        detect_mouse_mode(b"\x1b[?1006h", &mut mode);
        assert!(mode);
    }

    #[test]
    fn test_convert_color() {
        assert_eq!(convert_color(vt100::Color::Default), Color::Reset);
        assert_eq!(convert_color(vt100::Color::Idx(1)), Color::Indexed(1));
        assert_eq!(
            convert_color(vt100::Color::Rgb(255, 0, 0)),
            Color::Rgb(255, 0, 0)
        );
    }

    #[test]
    fn test_title_bar_click_detach() {
        let (_cols, _) = (80u16, 24u16);
        // Simulate: buttons are " [_] [x] " at end
        // width=80, buttons=" [_] [x] " len=9
        // buttons_start = 80-9 = 71
        // detach_start = 72, detach_end = 75
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 72,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let result = check_title_bar_click(mouse, 24, "test");
        assert!(matches!(result, Some(TitleBarAction::Detach)));
    }

    #[test]
    fn test_title_bar_click_stop() {
        // stop_start = 76, stop_end = 79
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 76,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let result = check_title_bar_click(mouse, 24, "test");
        assert!(matches!(result, Some(TitleBarAction::Stop)));
    }

    #[test]
    fn test_title_bar_click_name_area() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let result = check_title_bar_click(mouse, 24, "test");
        assert!(result.is_none());
    }

    #[test]
    fn test_title_bar_click_not_row_0() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 72,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        let result = check_title_bar_click(mouse, 24, "test");
        assert!(result.is_none());
    }
}
