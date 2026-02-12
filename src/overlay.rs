use anyhow::{Context, Result};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::{execute, terminal};
use nix::libc;
use nix::pty::openpty;
use std::io::{self, Read, Write};
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

/// Run a Docker session with a TUI overlay that shows a bottom status bar.
///
/// Uses a transparent PTY proxy: Docker output is forwarded as raw bytes to stdout.
/// The bottom terminal row is reserved for a status bar using ANSI scroll regions.
pub fn run_with_overlay(
    session_name: &str,
    title_color: Option<&str>,
    spawn_docker: impl FnOnce(RawFd) -> Result<Child>,
) -> Result<OverlayResult> {
    let fg_color = parse_color_ansi(title_color);

    let (term_cols, term_rows) = terminal::size().context("Failed to get terminal size")?;
    if term_rows < 3 || term_cols < 20 {
        anyhow::bail!("Terminal too small (need at least 20x3)");
    }

    let content_rows = term_rows - 1;

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

    let mut child = spawn_docker(slave_fd)?;
    drop(pty.slave);

    // Reader thread: reads PTY master output, sends to main thread
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let master_read_fd = unsafe { libc::dup(master_fd) };
    if master_read_fd < 0 {
        anyhow::bail!("Failed to dup master fd");
    }
    let _reader_thread = std::thread::spawn(move || {
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

    // Enter raw mode + mouse capture (NO alternate screen)
    let mut stdout = io::stdout();
    execute!(stdout, EnableMouseCapture)?;
    terminal::enable_raw_mode()?;

    // Clear screen and set scroll region to reserve bottom row for status bar
    write!(stdout, "\x1b[2J\x1b[H\x1b[1;{}r", content_rows)?;
    stdout.flush()?;

    let _guard = TerminalGuard;

    let mut app_cursor = false;
    let mut mouse_mode = false;
    let mut ctrl_p_pressed = false;

    // Draw initial status bar
    draw_status_bar(&mut stdout, term_rows, term_cols, session_name, &fg_color)?;

    let result = run_event_loop(
        &mut stdout,
        &mut child,
        &rx,
        master_fd,
        session_name,
        &fg_color,
        &mut ctrl_p_pressed,
        &mut mouse_mode,
        &mut app_cursor,
    );

    // Close master fd so the reader thread's read() gets EIO.
    // Don't join the reader thread — on macOS, read() on the dup'd master
    // may not return immediately when the slave closes.  The thread will
    // exit on its own (or be cleaned up at process exit, which is imminent).
    drop(pty.master);

    result
}

#[allow(clippy::too_many_arguments)]
fn run_event_loop(
    stdout: &mut io::Stdout,
    child: &mut Child,
    rx: &mpsc::Receiver<Vec<u8>>,
    master_fd: RawFd,
    session_name: &str,
    fg_color: &str,
    ctrl_p_pressed: &mut bool,
    mouse_mode: &mut bool,
    app_cursor: &mut bool,
) -> Result<OverlayResult> {
    let mut reader_done = false;
    let mut buf = Vec::with_capacity(8192);
    loop {
        // Drain PTY output into a buffer
        buf.clear();
        loop {
            match rx.try_recv() {
                Ok(data) => {
                    detect_mouse_mode(&data, mouse_mode);
                    detect_app_cursor_mode(&data, app_cursor);
                    buf.extend_from_slice(&data);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    reader_done = true;
                    break;
                }
            }
        }

        // Write buffered output + status bar as a single synchronized frame
        if !buf.is_empty() {
            let (cols, rows) = terminal::size().unwrap_or((80, 24));
            // Begin synchronized update (terminals that don't support it ignore this)
            stdout.write_all(b"\x1b[?2026h")?;
            stdout.write_all(&buf)?;
            draw_status_bar(stdout, rows, cols, session_name, fg_color)?;
            // End synchronized update — terminal renders the whole frame at once
            stdout.write_all(b"\x1b[?2026l")?;
            stdout.flush()?;
        }

        // Check if child exited
        if reader_done {
            // Reader thread exited — PTY slave is closed, child is exiting.
            // Use blocking wait to avoid spinning.
            let status = child.wait()?;
            stdout.flush()?;
            return Ok(OverlayResult::Exited(status.code().unwrap_or(1)));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // Drain remaining output
                while let Ok(data) = rx.try_recv() {
                    stdout.write_all(&data)?;
                }
                stdout.flush()?;
                return Ok(OverlayResult::Exited(status.code().unwrap_or(1)));
            }
            Ok(None) => {}
            Err(_) => return Ok(OverlayResult::Exited(1)),
        }

        // Poll for events with timeout for ~60fps
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
                        continue;
                    }

                    if *ctrl_p_pressed {
                        *ctrl_p_pressed = false;
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Char('Q') => {
                                return Ok(OverlayResult::Detached);
                            }
                            KeyCode::Char('x') | KeyCode::Char('X') => {
                                docker::stop_container(session_name);
                                let _ = child.wait();
                                return Ok(OverlayResult::Stopped);
                            }
                            _ => {
                                // Not a recognized combo — send the buffered Ctrl+P then this key
                                write_to_pty(master_fd, &[0x10]);
                                let bytes = key_to_bytes(key, *app_cursor);
                                write_to_pty(master_fd, &bytes);
                                continue;
                            }
                        }
                    }

                    if key.code == KeyCode::Char('p')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        *ctrl_p_pressed = true;
                        continue;
                    }

                    let bytes = key_to_bytes(key, *app_cursor);
                    write_to_pty(master_fd, &bytes);
                }
                Event::Mouse(mouse) => {
                    if !*mouse_mode {
                        continue;
                    }
                    // No row offset needed — status bar is at the bottom,
                    // so Docker's coordinate space maps directly.
                    if let Some(seq) = encode_mouse_event(mouse.kind, mouse.column, mouse.row) {
                        write_to_pty(master_fd, seq.as_bytes());
                    }
                }
                Event::Resize(cols, rows) => {
                    if rows > 1 {
                        let content_rows = rows - 1;
                        set_pty_size(master_fd, content_rows, cols);
                        stdout.write_all(b"\x1b[?2026h")?;
                        draw_status_bar(stdout, rows, cols, session_name, fg_color)?;
                        stdout.write_all(b"\x1b[?2026l")?;
                        stdout.flush()?;
                    }
                }
                _ => {}
            }
        }
    }
}

/// Draw the status bar on the bottom row using raw ANSI escapes.
///
/// Sequence: save cursor → move to bottom row → draw styled text →
/// re-assert scroll region → restore cursor.
///
/// The scroll region command (`\e[1;Nr`) is included here because it resets
/// the cursor to home (1,1) as a side effect.  By issuing it between
/// save (`\e7`) and restore (`\e8`), the cursor position is preserved.
fn draw_status_bar(
    stdout: &mut io::Stdout,
    rows: u16,
    cols: u16,
    session_name: &str,
    fg_color: &str,
) -> Result<()> {
    let content_rows = rows.saturating_sub(1).max(1);
    let width = cols as usize;
    let right = " ctrl+p,q:detach | ctrl+p,x:stop ";
    let right_len = right.len();

    let name_max = width.saturating_sub(right_len + 2);
    let display_name = if session_name.len() > name_max {
        &session_name[..name_max]
    } else {
        session_name
    };
    let left = format!(" {}", display_name);
    let left_len = left.len();
    let pad = width.saturating_sub(left_len + right_len);

    // \x1b7              = save cursor position
    // \x1b[{rows};1H     = move to bottom row (status bar)
    // \x1b[2K             = clear entire line (removes stale content after resize)
    // \x1b[{fg};1;100m   = bold + fg color + dark gray bg
    // ... bar content ...
    // \x1b[0m             = reset SGR attributes
    // \x1b[1;{N}r         = re-assert scroll region (resets cursor to 1,1)
    // \x1b8               = restore cursor to saved position
    write!(
        stdout,
        "\x1b7\x1b[{};1H\x1b[2K\x1b[{}1;100m{}{}{}\x1b[0m\x1b[1;{}r\x1b8",
        rows,
        fg_color,
        left,
        " ".repeat(pad),
        right,
        content_rows,
    )?;
    Ok(())
}

/// Detect whether the inner application wants application cursor key mode.
///
/// Scans for DECCKM set/reset: `\e[?1h` (enable) / `\e[?1l` (disable).
fn detect_app_cursor_mode(data: &[u8], app_cursor: &mut bool) {
    for window in data.windows(5) {
        if window == b"\x1b[?1h" {
            *app_cursor = true;
        } else if window == b"\x1b[?1l" {
            *app_cursor = false;
        }
    }
}

/// Detect whether the inner application is requesting mouse tracking.
fn detect_mouse_mode(data: &[u8], mouse_mode: &mut bool) {
    let s = match std::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => return,
    };
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

/// Encode a crossterm mouse event as an SGR (1006) escape sequence.
///
/// No row offset is needed since the status bar is at the bottom.
fn encode_mouse_event(kind: MouseEventKind, col: u16, row: u16) -> Option<String> {
    let (button, suffix) = match kind {
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
    Some(format!(
        "\x1b[<{};{};{}{}",
        button,
        col + 1,
        row + 1,
        suffix
    ))
}

/// Parse a color name/hex into an ANSI SGR foreground code string.
///
/// Returns a string like "37;" (white fg) or "38;2;R;G;B;" (true color fg).
fn parse_color_ansi(color: Option<&str>) -> String {
    match color {
        None => "37;".to_string(), // white
        Some(s) => {
            // Try hex color
            if let Some(hex) = s.strip_prefix('#') {
                if hex.len() == 6 {
                    if let (Ok(r), Ok(g), Ok(b)) = (
                        u8::from_str_radix(&hex[0..2], 16),
                        u8::from_str_radix(&hex[2..4], 16),
                        u8::from_str_radix(&hex[4..6], 16),
                    ) {
                        return format!("38;2;{};{};{};", r, g, b);
                    }
                }
            }
            // Named colors → SGR foreground codes
            match s.to_lowercase().as_str() {
                "black" => "30;",
                "red" => "31;",
                "green" => "32;",
                "yellow" => "33;",
                "blue" => "34;",
                "magenta" => "35;",
                "cyan" => "36;",
                "white" => "37;",
                "gray" | "grey" => "90;",
                "darkgray" | "darkgrey" => "90;",
                "lightred" => "91;",
                "lightgreen" => "92;",
                "lightyellow" => "93;",
                "lightblue" => "94;",
                "lightmagenta" => "95;",
                "lightcyan" => "96;",
                _ => "37;",
            }
            .to_string()
        }
    }
}

fn key_to_bytes(key: KeyEvent, app_cursor: bool) -> Vec<u8> {
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
            let mut bytes = vec![0x1b];
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

/// RAII guard for terminal cleanup.
///
/// Restores: scroll region, raw mode, mouse capture.
/// Does NOT leave alternate screen (we never entered it).
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, DisableMouseCapture);
        // Reset scroll region to full terminal
        let _ = write!(stdout, "\x1b[r");
        // Move to the actual bottom row, clear the status bar remnant, then newline
        // so the shell prompt starts on a clean line.
        // Use 999 to reliably reach the bottom regardless of current terminal size.
        let _ = writeln!(stdout, "\x1b[999;1H\x1b[2K");
        let _ = stdout.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::MouseEventKind;

    #[test]
    fn test_parse_color_ansi_none() {
        assert_eq!(parse_color_ansi(None), "37;");
    }

    #[test]
    fn test_parse_color_ansi_named() {
        assert_eq!(parse_color_ansi(Some("blue")), "34;");
        assert_eq!(parse_color_ansi(Some("Blue")), "34;");
        assert_eq!(parse_color_ansi(Some("RED")), "31;");
        assert_eq!(parse_color_ansi(Some("green")), "32;");
        assert_eq!(parse_color_ansi(Some("yellow")), "33;");
        assert_eq!(parse_color_ansi(Some("cyan")), "36;");
        assert_eq!(parse_color_ansi(Some("magenta")), "35;");
        assert_eq!(parse_color_ansi(Some("white")), "37;");
        assert_eq!(parse_color_ansi(Some("black")), "30;");
        assert_eq!(parse_color_ansi(Some("gray")), "90;");
        assert_eq!(parse_color_ansi(Some("grey")), "90;");
    }

    #[test]
    fn test_parse_color_ansi_hex() {
        assert_eq!(parse_color_ansi(Some("#ff0000")), "38;2;255;0;0;");
        assert_eq!(parse_color_ansi(Some("#00ff00")), "38;2;0;255;0;");
        assert_eq!(parse_color_ansi(Some("#0000ff")), "38;2;0;0;255;");
        assert_eq!(parse_color_ansi(Some("#abcdef")), "38;2;171;205;239;");
    }

    #[test]
    fn test_parse_color_ansi_invalid_hex() {
        assert_eq!(parse_color_ansi(Some("#fff")), "37;");
        assert_eq!(parse_color_ansi(Some("#gggggg")), "37;");
    }

    #[test]
    fn test_parse_color_ansi_unknown() {
        assert_eq!(parse_color_ansi(Some("unknown")), "37;");
    }

    #[test]
    fn test_key_to_bytes_char() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(key_to_bytes(key, false), vec![b'a']);
    }

    #[test]
    fn test_key_to_bytes_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(key_to_bytes(key, false), vec![b'\r']);
    }

    #[test]
    fn test_key_to_bytes_ctrl_c() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(key_to_bytes(key, false), vec![3]);
    }

    #[test]
    fn test_key_to_bytes_arrow_keys() {
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(key_to_bytes(up, false), vec![0x1b, b'[', b'A']);
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(key_to_bytes(down, false), vec![0x1b, b'[', b'B']);
    }

    #[test]
    fn test_key_to_bytes_arrow_keys_app_cursor() {
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(key_to_bytes(up, true), vec![0x1b, b'O', b'A']);
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(key_to_bytes(down, true), vec![0x1b, b'O', b'B']);
    }

    #[test]
    fn test_key_to_bytes_esc() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(key_to_bytes(key, false), vec![0x1b]);
    }

    #[test]
    fn test_key_to_bytes_backspace() {
        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(key_to_bytes(key, false), vec![0x7f]);
    }

    #[test]
    fn test_key_to_bytes_f_keys() {
        let f1 = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
        assert_eq!(key_to_bytes(f1, false), vec![0x1b, b'O', b'P']);
        let f5 = KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE);
        assert_eq!(
            key_to_bytes(f5, false),
            vec![0x1b, b'[', b'1', b'5', b'~']
        );
    }

    #[test]
    fn test_key_to_bytes_alt_char() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT);
        assert_eq!(key_to_bytes(key, false), vec![0x1b, b'x']);
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
    fn test_detect_app_cursor_mode_enable() {
        let mut mode = false;
        detect_app_cursor_mode(b"\x1b[?1h", &mut mode);
        assert!(mode);
    }

    #[test]
    fn test_detect_app_cursor_mode_disable() {
        let mut mode = true;
        detect_app_cursor_mode(b"\x1b[?1l", &mut mode);
        assert!(!mode);
    }

    #[test]
    fn test_detect_app_cursor_mode_short_data() {
        let mut mode = false;
        detect_app_cursor_mode(b"\x1b[", &mut mode);
        assert!(!mode); // unchanged
    }

    #[test]
    fn test_encode_mouse_event_left_click() {
        let seq = encode_mouse_event(MouseEventKind::Down(MouseButton::Left), 10, 5);
        assert_eq!(seq, Some("\x1b[<0;11;6M".to_string()));
    }

    #[test]
    fn test_encode_mouse_event_right_click() {
        let seq = encode_mouse_event(MouseEventKind::Down(MouseButton::Right), 0, 0);
        assert_eq!(seq, Some("\x1b[<2;1;1M".to_string()));
    }

    #[test]
    fn test_encode_mouse_event_release() {
        let seq = encode_mouse_event(MouseEventKind::Up(MouseButton::Left), 5, 3);
        assert_eq!(seq, Some("\x1b[<0;6;4m".to_string()));
    }

    #[test]
    fn test_encode_mouse_event_scroll() {
        let seq = encode_mouse_event(MouseEventKind::ScrollUp, 10, 10);
        assert_eq!(seq, Some("\x1b[<64;11;11M".to_string()));
    }
}
