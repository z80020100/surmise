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
//! crossterm and so does the question `column` asks.

use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::ffi::CStr;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, AsRawFd};
use std::time::Duration;

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

/// Whether stdin is a terminal rather than a pipe.
///
/// `pick::run` reads the widget's own record off stdin before it ever gets
/// here. A terminal on stdin means nobody piped that record in and nobody
/// is about to type it either, which is exactly the shape of a hand run of
/// `surmise --pick`. Reading anyway would block on a key that never comes.
pub fn stdin_is_terminal() -> bool {
    // SAFETY: `isatty` reads the descriptor and touches no memory of ours.
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
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

/// Ask the terminal where the cursor is. Give up when it does not answer.
///
/// Raw mode has to be on already. crossterm asks and reads the reply through
/// the reader `pick` takes every key from. A key pressed before the reply
/// lands therefore waits in that reader for the loop rather than going with
/// the reply. A terminal that never answers costs the two seconds crossterm
/// waits and the keys pressed inside them arrive after it.
///
/// crossterm asks on stdout and stdout is the widget's pipe. The terminal
/// stands in for it for the length of the call. This is therefore not a call
/// to make from two threads either.
pub fn column(term: &File) -> io::Result<Option<usize>> {
    // crossterm asks again for as long as its reader fails and a reader that
    // never started fails every time. `pick` reads every key through that
    // same reader and the error ends it here instead.
    crossterm::event::poll(Duration::ZERO)?;
    let pipe = io::stdout().as_fd().try_clone_to_owned()?;
    // SAFETY: both descriptors are live for the call.
    if unsafe { libc::dup2(term.as_raw_fd(), libc::STDOUT_FILENO) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let pos = crossterm::cursor::position();
    // SAFETY: `pipe` owns its descriptor until the end of this function. The
    // line goes out on stdout at the end and a pipe that did not come back
    // would send it to the terminal and hand the widget an empty line. An
    // error leaves the widget the line it already has.
    if unsafe { libc::dup2(pipe.as_raw_fd(), libc::STDOUT_FILENO) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(pos.ok().map(|(col, _)| usize::from(col)))
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
}
