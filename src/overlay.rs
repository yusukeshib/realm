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
    let (fg_color, bg_color) = parse_status_colors(title_color);

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
    let mut cursor_visible = true;
    let mut ctrl_p_pressed = false;

    // Draw initial status bar
    draw_status_bar(&mut stdout, term_rows, term_cols, session_name, &fg_color, &bg_color, cursor_visible)?;
    stdout.flush()?;

    // Nudge the inner app to redraw by toggling the PTY size.
    // TIOCSWINSZ sends SIGWINCH to the foreground process group, which makes
    // shells and TUI apps repaint.  This is needed because we just cleared the
    // screen and the inner app doesn't know it should redraw.
    set_pty_size(master_fd, content_rows.saturating_sub(1).max(1), term_cols);
    set_pty_size(master_fd, content_rows, term_cols);

    let result = run_event_loop(
        &mut stdout,
        &mut child,
        &rx,
        master_fd,
        session_name,
        &fg_color,
        &bg_color,
        &mut ctrl_p_pressed,
        &mut mouse_mode,
        &mut app_cursor,
        &mut cursor_visible,
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
    bg_color: &str,
    ctrl_p_pressed: &mut bool,
    mouse_mode: &mut bool,
    app_cursor: &mut bool,
    cursor_visible: &mut bool,
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
                    detect_cursor_visible(&data, cursor_visible);
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
            draw_status_bar(stdout, rows, cols, session_name, fg_color, bg_color, *cursor_visible)?;
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
                Event::Resize(mut cols, mut rows) => {
                    // Drain queued resize events so we only act on the final size
                    while event::poll(Duration::ZERO)? {
                        match event::read()? {
                            Event::Resize(c, r) => {
                                cols = c;
                                rows = r;
                            }
                            _ => break,
                        }
                    }
                    if rows > 1 {
                        let content_rows = rows - 1;
                        // Update scroll region and redraw status bar — don't clear the
                        // screen so the old content stays visible until the inner app
                        // repaints (avoids blank flash).
                        stdout.write_all(b"\x1b[?2026h")?;
                        write!(stdout, "\x1b[1;{}r", content_rows)?;
                        draw_status_bar(stdout, rows, cols, session_name, fg_color, bg_color, *cursor_visible)?;
                        stdout.write_all(b"\x1b[?2026l")?;
                        stdout.flush()?;
                        // Nudge inner app to redraw by toggling PTY size
                        set_pty_size(master_fd, content_rows.saturating_sub(1).max(1), cols);
                        set_pty_size(master_fd, content_rows, cols);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Draw the status bar on the bottom row using raw ANSI escapes.
///
/// Sequence: save cursor (SCP) → move to bottom row → draw styled text →
/// re-assert scroll region → restore cursor (RCP).
///
/// Uses SCP/RCP (`\e[s`/`\e[u`) instead of DECSC/DECRC (`\e7`/`\e8`) so
/// we don't clobber the inner application's cursor save slot.  The scroll
/// region command (`\e[1;Nr`) resets the cursor to home (1,1) as a side
/// effect, but the RCP restore undoes that.
fn draw_status_bar(
    stdout: &mut io::Stdout,
    rows: u16,
    cols: u16,
    session_name: &str,
    fg_color: &str,
    bg_color: &str,
    cursor_visible: bool,
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

    // Hide cursor during status bar draw so it never blinks at intermediate
    // positions (e.g. the status bar row).
    //
    // Uses SCP/RCP (\x1b[s / \x1b[u) instead of DECSC/DECRC (\x1b7 / \x1b8)
    // to avoid clobbering the inner application's cursor save slot.
    //
    // After restoring cursor, only re-show it if the inner app had it visible.
    let cursor_suffix = if cursor_visible { "\x1b[?25h" } else { "" };
    write!(
        stdout,
        "\x1b[?25l\x1b[s\x1b[{};1H\x1b[2K\x1b[{}{}1m{}{}{}\x1b[0m\x1b[1;{}r\x1b[u{}",
        rows,
        fg_color,
        bg_color,
        left,
        " ".repeat(pad),
        right,
        content_rows,
        cursor_suffix,
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

/// Detect whether the inner application wants the cursor visible or hidden.
///
/// Scans for DECTCEM set/reset: `\e[?25h` (show) / `\e[?25l` (hide).
fn detect_cursor_visible(data: &[u8], visible: &mut bool) {
    for window in data.windows(6) {
        if window[..5] == *b"\x1b[?25" {
            if window[5] == b'h' {
                *visible = true;
            } else if window[5] == b'l' {
                *visible = false;
            }
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

/// Parse a color name/hex into foreground and background SGR code fragments.
///
/// Returns `(fg, bg)` where each is a string like `"97;"` or `"48;2;R;G;B;"`.
/// The background is set from the user's color choice; the foreground is
/// auto-picked (black or bright-white) based on the background's luminance.
///
/// Default (no color): bright white on bright black (`"97;"`, `"100;"`).
fn parse_status_colors(color: Option<&str>) -> (String, String) {
    match color {
        None => ("97;".to_string(), "100;".to_string()),
        Some(s) => {
            // Try hex color
            if let Some(hex) = s.strip_prefix('#') {
                if hex.len() == 6 {
                    if let (Ok(r), Ok(g), Ok(b)) = (
                        u8::from_str_radix(&hex[0..2], 16),
                        u8::from_str_radix(&hex[2..4], 16),
                        u8::from_str_radix(&hex[4..6], 16),
                    ) {
                        let fg = luminance_fg(r, g, b);
                        return (fg.to_string(), format!("48;2;{};{};{};", r, g, b));
                    }
                }
            }
            // Named colors → SGR background codes + RGB for luminance
            let (bg, r, g, b) = match s.to_lowercase().as_str() {
                "black" => ("40;", 0u8, 0u8, 0u8),
                "red" => ("41;", 170, 0, 0),
                "green" => ("42;", 0, 170, 0),
                "yellow" => ("43;", 170, 170, 0),
                "blue" => ("44;", 0, 0, 170),
                "magenta" => ("45;", 170, 0, 170),
                "cyan" => ("46;", 0, 170, 170),
                "white" => ("47;", 170, 170, 170),
                "gray" | "grey" => ("100;", 85, 85, 85),
                "lightred" => ("101;", 255, 85, 85),
                "lightgreen" => ("102;", 85, 255, 85),
                "lightyellow" => ("103;", 255, 255, 85),
                "lightblue" => ("104;", 85, 85, 255),
                "lightmagenta" => ("105;", 255, 85, 255),
                "lightcyan" => ("106;", 85, 255, 255),
                _ => return ("97;".to_string(), "100;".to_string()),
            };
            let fg = luminance_fg(r, g, b);
            (fg.to_string(), bg.to_string())
        }
    }
}

/// Pick black or bright-white foreground based on perceived luminance.
fn luminance_fg(r: u8, g: u8, b: u8) -> &'static str {
    let l = 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;
    if l >= 128.0 { "30;" } else { "97;" }
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
    fn test_parse_status_colors_none() {
        // Default: bright white on bright black
        assert_eq!(parse_status_colors(None), ("97;".to_string(), "100;".to_string()));
    }

    #[test]
    fn test_parse_status_colors_named() {
        // Dark backgrounds → white text
        assert_eq!(parse_status_colors(Some("blue")), ("97;".to_string(), "44;".to_string()));
        assert_eq!(parse_status_colors(Some("Blue")), ("97;".to_string(), "44;".to_string()));
        assert_eq!(parse_status_colors(Some("RED")), ("97;".to_string(), "41;".to_string()));
        assert_eq!(parse_status_colors(Some("black")), ("97;".to_string(), "40;".to_string()));
        assert_eq!(parse_status_colors(Some("magenta")), ("97;".to_string(), "45;".to_string()));
        assert_eq!(parse_status_colors(Some("gray")), ("97;".to_string(), "100;".to_string()));
        assert_eq!(parse_status_colors(Some("grey")), ("97;".to_string(), "100;".to_string()));
        // Light backgrounds → black text
        assert_eq!(parse_status_colors(Some("white")), ("30;".to_string(), "47;".to_string()));
        assert_eq!(parse_status_colors(Some("lightgreen")), ("30;".to_string(), "102;".to_string()));
        assert_eq!(parse_status_colors(Some("lightcyan")), ("30;".to_string(), "106;".to_string()));
    }

    #[test]
    fn test_parse_status_colors_named_luminance() {
        // yellow: L = 0.299*170 + 0.587*170 + 0.114*0 = 150.6 → black text
        assert_eq!(parse_status_colors(Some("yellow")), ("30;".to_string(), "43;".to_string()));
        // green: L = 0.299*0 + 0.587*170 + 0.114*0 = 99.8 → white text
        assert_eq!(parse_status_colors(Some("green")), ("97;".to_string(), "42;".to_string()));
        // cyan: L = 0.299*0 + 0.587*170 + 0.114*170 = 119.2 → white text
        assert_eq!(parse_status_colors(Some("cyan")), ("97;".to_string(), "46;".to_string()));
    }

    #[test]
    fn test_parse_status_colors_hex() {
        // Dark hex → white text
        assert_eq!(parse_status_colors(Some("#ff0000")), ("97;".to_string(), "48;2;255;0;0;".to_string()));
        assert_eq!(parse_status_colors(Some("#0000ff")), ("97;".to_string(), "48;2;0;0;255;".to_string()));
        // Light hex → black text
        assert_eq!(parse_status_colors(Some("#ffffff")), ("30;".to_string(), "48;2;255;255;255;".to_string()));
        assert_eq!(parse_status_colors(Some("#00ff00")), ("30;".to_string(), "48;2;0;255;0;".to_string()));
    }

    #[test]
    fn test_parse_status_colors_invalid_hex() {
        // Falls back to default
        assert_eq!(parse_status_colors(Some("#fff")), ("97;".to_string(), "100;".to_string()));
        assert_eq!(parse_status_colors(Some("#gggggg")), ("97;".to_string(), "100;".to_string()));
    }

    #[test]
    fn test_parse_status_colors_unknown() {
        assert_eq!(parse_status_colors(Some("unknown")), ("97;".to_string(), "100;".to_string()));
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
