//! The terminal device.
//!
//! A shell widget runs surmise inside a command substitution. That makes stdout
//! a pipe and leaves stdin as whatever the widget put there. Opening `/dev/tty`
//! looks like the obvious answer and is not: on macOS a descriptor obtained
//! from `/dev/tty` cannot be registered with kqueue and the event reader fails
//! with EINVAL. The real device behind the terminal has to be resolved and
//! opened by name.
//!
//! The device's mode lives here too. `Raw` holds the terminal in raw mode and
//! in bracketed paste for as long as surmise wants both. Both calls come from
//! crossterm. `column` names the one crossterm function this module replaces
//! rather than calls.

use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::ffi::CStr;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

/// The device name behind `fd`. `None` when `fd` is not a terminal.
fn name_of(fd: i32) -> Option<String> {
    // SAFETY: `isatty` reads the descriptor and touches no memory of ours.
    if unsafe { libc::isatty(fd) } != 1 {
        return None;
    }
    let mut buf = [0; libc::PATH_MAX as usize];
    // SAFETY: the buffer is live for the call and the length is its own. The
    // reentrant form is the one used, because plain `ttyname` answers out of a
    // static buffer that the next call overwrites.
    let err = unsafe { libc::ttyname_r(fd, buf.as_mut_ptr(), buf.len()) };
    if err != 0 {
        return None;
    }
    // SAFETY: `ttyname_r` returning zero means the buffer holds a terminated
    // string.
    let name = unsafe { CStr::from_ptr(buf.as_ptr()) };
    Some(name.to_string_lossy().into_owned())
}

/// Resolve the terminal and put it on stdin. The event reader then has a
/// pollable descriptor. The returned handle writes to the same device.
///
/// This replaces the process's own stdin and is therefore not a call to make
/// from two threads.
pub fn claim() -> io::Result<File> {
    // `SURMISE_TTY` is the seam a test drives surmise through. It names a pty
    // the test opened itself. Otherwise the descriptors are asked in turn with
    // stdout last, because it is the command substitution's pipe.
    let path = std::env::var("SURMISE_TTY")
        .ok()
        .filter(|p| !p.is_empty())
        .or_else(|| name_of(libc::STDIN_FILENO))
        .or_else(|| name_of(libc::STDERR_FILENO))
        .or_else(|| name_of(libc::STDOUT_FILENO))
        .unwrap_or_else(|| "/dev/tty".to_string());

    let dev = OpenOptions::new().read(true).write(true).open(&path)?;
    // SAFETY: `dev` owns the descriptor for the whole call.
    if unsafe { libc::dup2(dev.as_raw_fd(), libc::STDIN_FILENO) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // `dup2` gave stdin a descriptor of its own. This handle therefore stays
    // usable and closing it later leaves stdin alone.
    Ok(dev)
}

/// How long to wait for a terminal to say where the cursor is.
const DSR_WAIT: Duration = Duration::from_millis(120);

fn was_interrupted() -> bool {
    io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
}

/// Whether the device has bytes waiting inside `wait`. `None` when a signal
/// cut the wait short before either answer.
fn readable_within(wait: Duration) -> Option<bool> {
    let mut p = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = wait.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: the pointer is to one live `pollfd` and the count says one.
    let n = unsafe { libc::poll(&mut p, 1, ms) };
    if n < 0 && was_interrupted() {
        return None;
    }
    Some(n > 0)
}

/// The zero-based column out of `\x1b[{row};{col}R`.
///
/// The last CSI in the buffer wins. Anything typed before surmise opened is
/// still in the terminal's queue and arrives ahead of the reply. A stray `R`
/// or `[` in it would otherwise be read as the answer.
fn parse_dsr(buf: &[u8]) -> Option<usize> {
    let start = buf.windows(2).rposition(|w| w == b"\x1b[")? + 2;
    let end = start + buf[start..].iter().position(|b| *b == b'R')?;
    // The row is dropped rather than parsed. That also carries the leading `?`
    // some terminals answer with.
    let text = std::str::from_utf8(&buf[start..end]).ok()?;
    let (_, col) = text.split_once(';')?;
    col.trim().parse::<usize>().ok()?.checked_sub(1)
}

/// Ask the terminal where the cursor is. Give up when it does not answer.
///
/// Raw mode has to be on already and nothing else may be reading the terminal
/// yet. The reply arrives on stdin rather than on `out`, because `claim` put
/// the device there. crossterm's own `cursor::position` asks the same question
/// and then waits forever on a terminal that never replies. This exists for
/// that reason.
///
/// A key pressed inside the wait is lost. That window is 120 ms and it opens
/// before there is anything on screen to type at.
pub fn column(out: &mut File) -> Option<usize> {
    out.write_all(b"\x1b[6n").ok()?;
    out.flush().ok()?;

    let deadline = Instant::now() + DSR_WAIT;
    let mut buf = Vec::new();
    while buf.len() < 64 {
        let left = deadline.checked_duration_since(Instant::now())?;
        // A signal can cut a wait or a read short. The deadline still governs
        // and the loop therefore asks again rather than giving up.
        let Some(ready) = readable_within(left) else {
            continue;
        };
        if !ready {
            return None;
        }
        let mut chunk = [0u8; 32];
        // SAFETY: the pointer is to `chunk` and the count is its length.
        let n = unsafe {
            libc::read(
                libc::STDIN_FILENO,
                chunk.as_mut_ptr().cast::<libc::c_void>(),
                chunk.len(),
            )
        };
        if n < 0 && was_interrupted() {
            continue;
        }
        if n <= 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n as usize]);
        if let Some(col) = parse_dsr(&buf) {
            return Some(col);
        }
    }
    None
}

/// The terminal is surmise's for as long as this value lives. A panic or an
/// error on the way out would otherwise hand the shell back a terminal still
/// in raw mode and still in bracketed paste.
///
/// Raw mode belongs to the process rather than to the handle. One guard at a
/// time is therefore the rule. A second would hand the terminal back the
/// moment the first of them drops.
pub struct Raw(File);

impl Raw {
    /// `dev` is a handle of the guard's own. Dropping the guard closes it and
    /// leaves the caller's handle alone.
    pub fn on(dev: File) -> io::Result<Raw> {
        enable_raw_mode()?;
        // The guard exists before anything else can fail. Raw mode is given
        // back even when what follows does not finish.
        let mut raw = Raw(dev);
        let _ = crossterm::execute!(raw.0, EnableBracketedPaste);
        Ok(raw)
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = crossterm::execute!(self.0, DisableBracketedPaste);
        let _ = disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_descriptor_that_is_not_a_terminal_has_no_name() {
        let f = File::open("/dev/null").expect("/dev/null opens");
        assert_eq!(name_of(f.as_raw_fd()), None);
    }

    #[test]
    fn a_reply_gives_the_column_one_lower_than_it_reports() {
        assert_eq!(parse_dsr(b"\x1b[12;34R"), Some(33));
    }

    #[test]
    fn the_leftmost_column_is_zero() {
        assert_eq!(parse_dsr(b"\x1b[1;1R"), Some(0));
    }

    #[test]
    fn a_column_below_one_is_not_a_column() {
        assert_eq!(parse_dsr(b"\x1b[1;0R"), None);
    }

    #[test]
    fn a_reply_that_has_not_arrived_in_full_is_not_read_early() {
        assert_eq!(parse_dsr(b"\x1b[12;34"), None);
        assert_eq!(parse_dsr(b"\x1b["), None);
        assert_eq!(parse_dsr(b"\x1b"), None);
        assert_eq!(parse_dsr(b""), None);
    }

    #[test]
    fn a_terminal_that_answers_with_a_question_mark_is_still_read() {
        assert_eq!(parse_dsr(b"\x1b[?12;34R"), Some(33));
    }

    #[test]
    fn a_keystroke_waiting_ahead_of_the_reply_is_stepped_over() {
        assert_eq!(parse_dsr(b"R[9;9Rx\x1b[12;34R"), Some(33));
    }

    #[test]
    fn a_body_that_is_not_a_position_is_refused() {
        assert_eq!(parse_dsr(b"\x1b[12;xR"), None);
        assert_eq!(parse_dsr(b"\x1b[34R"), None);
        assert_eq!(parse_dsr(b"12;34R"), None);
    }
}
