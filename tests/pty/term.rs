//! A terminal a test can type into and read a rendered screen out of.
//!
//! portable-pty opens the device and vt100 renders it. The program runs inside
//! the pty rather than beside it. crossterm resolves `/dev/tty` for raw mode
//! rather than reading stdin. A child spawned any other way would put the
//! terminal the suite was started from into raw mode.
//!
//! The screen is what the assertions read. A byte stream can carry a
//! box-drawing character and still render as garbage.

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{ErrorKind, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// How long one read waits before the caller gets its turn back.
const POLL: Duration = Duration::from_millis(20);

/// How long a hangup gets before the child is killed outright.
const REAP: Duration = Duration::from_secs(2);

/// A screen that answers a cursor-position report.
///
/// surmise asks where the cursor is so it can draw on the shell's own prompt
/// row. A screen that stays silent sends it down the fallback path and the
/// path a person actually gets would then go untested.
///
/// vt100 implements no CSI `n`. The query therefore arrives here instead and
/// the reply waits in the buffer until `Term` sends it, because a callback
/// holds the screen rather than the device.
#[derive(Default)]
struct Answering(Vec<u8>);

impl vt100::Callbacks for Answering {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        // A private marker arrives as the first intermediate. `\x1b[?6n` is
        // DECXCPR and wants a reply of its own rather than this one.
        if i1.is_some() || c != 'n' || !matches!(params, [[6]]) {
            return;
        }
        // The report is one-based and `cursor_position` is not.
        let (row, col) = screen.cursor_position();
        let reply = format!("\x1b[{};{}R", row + 1, col + 1);
        self.0.extend_from_slice(reply.as_bytes());
    }
}

/// A run of painted cells on one row: where it starts, where it ends and what
/// it holds.
pub struct Panel {
    pub row: u16,
    pub lo: u16,
    pub hi: u16,
    pub text: String,
}

pub struct Term {
    parser: vt100::Parser<Answering>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<Vec<u8>>,
    child: Box<dyn Child + Send + Sync>,
    /// Dropping this hangs the child up. It has no other use here.
    _master: Box<dyn MasterPty + Send>,
}

impl Term {
    /// Run `cmd` in a pty `cols` by `rows` and render what it draws.
    pub fn new(cmd: CommandBuilder, cols: u16, rows: u16) -> Term {
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system().openpty(size).expect("a pty opens");
        let child = pair.slave.spawn_command(cmd).expect("the program starts");
        // The slave goes now that the child holds one. Keeping it open would
        // hold the read below short of its end for as long as `Term` lived.
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("a reader");
        let writer = pair.master.take_writer().expect("a writer");
        // That read blocks and a test needs a deadline. A thread of its own
        // turns it into something `recv_timeout` can wait on.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 65536];
            loop {
                let read = reader.read(&mut buf);
                if matches!(&read, Err(e) if e.kind() == ErrorKind::Interrupted) {
                    // A signal can cut a read short. The stream has not ended
                    // and the loop therefore asks again.
                    continue;
                }
                let Ok(n) = read else { break };
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });

        Term {
            parser: vt100::Parser::new_with_callbacks(rows, cols, 0, Answering::default()),
            writer,
            rx,
            child,
            _master: pair.master,
        }
    }

    /// Send whatever the screen owes the program. A cursor-position report is
    /// the only answer it has so far.
    fn answer(&mut self) {
        let reply = std::mem::take(&mut self.parser.callbacks_mut().0);
        if !reply.is_empty() {
            let _ = self.writer.write_all(&reply);
            let _ = self.writer.flush();
        }
    }

    /// Read whatever is waiting onto the screen. `false` once the pty has
    /// nothing more to give. Every loop below wants this same read and they
    /// differ only in what they are waiting for.
    fn drain(&mut self) -> bool {
        match self.rx.recv_timeout(POLL) {
            Ok(data) => {
                self.parser.process(&data);
                self.answer();
                true
            }
            Err(RecvTimeoutError::Timeout) => true,
            Err(RecvTimeoutError::Disconnected) => false,
        }
    }

    /// Put `text` on the screen as though the shell had drawn it.
    ///
    /// The picker draws on the shell's own prompt row and works that row out
    /// from where the cursor stands. A test that starts on a bare screen
    /// therefore always takes the fallback path and the row a person really
    /// gets would go untested.
    pub fn shell_drew(&mut self, text: &str) {
        self.parser.process(text.as_bytes());
    }

    /// Type `keys` at the program.
    pub fn send(&mut self, keys: &str) {
        let taken = "the pty takes the keys";
        self.writer.write_all(keys.as_bytes()).expect(taken);
        self.writer.flush().expect(taken);
    }

    /// Read for `limit`, or until the pty ends.
    pub fn pump(&mut self, limit: Duration) {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline && self.drain() {}
    }

    /// Read until `done` says the screen holds what the caller waited for.
    /// `false` at the deadline or once the pty ends without it.
    fn wait_for(&mut self, limit: Duration, done: impl Fn(&Term) -> bool) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if done(self) {
                return true;
            }
            if !self.drain() {
                return false;
            }
        }
        false
    }

    /// Wait for surmise to paint. `false` when it never does.
    pub fn wait_panel(&mut self, limit: Duration) -> bool {
        self.wait_for(limit, |t| !t.panel().is_empty())
    }

    /// Wait for surmise to stop painting. `false` while it is still there.
    ///
    /// The picker erases its frame on the way out. A test that reads the
    /// screen before that lands sees the menu it just closed.
    pub fn wait_bare(&mut self, limit: Duration) -> bool {
        self.wait_for(limit, |t| t.panel().is_empty())
    }

    /// Wait for `text` to show on the screen. `false` when it never does.
    ///
    /// A shell has to start before a test can type at it and the prompt is
    /// the only thing that says it has.
    pub fn wait_line(&mut self, text: &str, limit: Duration) -> bool {
        self.wait_for(limit, |t| t.lines().iter().any(|l| l.contains(text)))
    }

    /// The exit status, once the program has one. `None` while it is still
    /// running at the deadline.
    ///
    /// The screen keeps being read throughout. A program that filled the pty's
    /// buffer would otherwise block on the write and never reach its exit.
    pub fn status(&mut self, limit: Duration) -> Option<u8> {
        // The picker's contract is a byte and its four statuses are 0 to 3.
        // The width the pty reports one in is its own business. `WEXITSTATUS`
        // is a byte anyway and portable-pty answers a signalled child with 1.
        let code = |s: portable_pty::ExitStatus| s.exit_code() as u8;
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if let Ok(Some(s)) = self.child.try_wait() {
                return Some(code(s));
            }
            if !self.drain() {
                // Nothing more will arrive and the receiver now answers at
                // once. Hold the pace so the deadline still governs.
                std::thread::sleep(POLL);
            }
        }
        None
    }

    /// The screen as text, with the trailing blank rows dropped.
    pub fn lines(&self) -> Vec<String> {
        let screen = self.parser.screen();
        let (_, cols) = screen.size();
        let mut out: Vec<String> = screen
            .rows(0, cols)
            .map(|row| row.trim_end().to_string())
            .collect();
        while out.last().is_some_and(String::is_empty) {
            out.pop();
        }
        out
    }

    /// Every run of cells surmise painted, row by row.
    ///
    /// surmise paints on a background of its own. Every cell it owns is
    /// therefore identifiable no matter what character is in it. Drawn borders
    /// would not be: the terminal renders a box-drawing character it has no
    /// glyph for as whatever it likes.
    pub fn panel(&self) -> Vec<Panel> {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let painted = |row: u16, col: u16| {
            screen
                .cell(row, col)
                .is_some_and(|c| c.bgcolor() != vt100::Color::Default)
        };
        (0..rows)
            .filter_map(|row| {
                let lo = (0..cols).find(|&col| painted(row, col))?;
                let hi = (lo..cols).rfind(|&col| painted(row, col))?;
                let text = (lo..=hi)
                    .filter_map(|col| screen.cell(row, col))
                    .map(vt100::Cell::contents)
                    .collect();
                Some(Panel { row, lo, hi, text })
            })
            .collect()
    }

    /// The characters in one panel row that `want` picks out.
    ///
    /// `at` counts the painted rows rather than the screen's. `want` reads a
    /// cell against the row's own first one, which carries whatever that row
    /// draws its plain text with.
    fn picked(&self, at: usize, want: impl Fn(&vt100::Cell, &vt100::Cell) -> bool) -> String {
        let screen = self.parser.screen();
        let panel = self.panel();
        let Some(p) = panel.get(at) else {
            return String::new();
        };
        let Some(plain) = screen.cell(p.row, p.lo) else {
            return String::new();
        };
        (p.lo..=p.hi)
            .filter_map(|col| screen.cell(p.row, col))
            .filter(|&c| want(c, plain))
            .map(vt100::Cell::contents)
            .collect()
    }

    /// The characters in one panel row that carry a ground of their own.
    ///
    /// Reading the colour is the only way to see a mark: the character under
    /// it is the same character either way.
    pub fn marks(&self, at: usize) -> String {
        self.picked(at, |c, plain| c.bgcolor() != plain.bgcolor())
    }

    /// The underlined characters in one panel row.
    pub fn underlined(&self, at: usize) -> String {
        self.picked(at, |c, _| c.underline())
    }
}

impl Drop for Term {
    /// Reap the child. A suite that left one behind per terminal would fill
    /// the process table before it finished.
    ///
    /// `kill` sends a hangup rather than a kill and a program is free to
    /// handle one. The wait is therefore bounded and a program still standing
    /// at the end of it gets the signal nothing can handle. A suite that hangs
    /// says far less than one that fails.
    fn drop(&mut self) {
        let _ = self.child.kill();
        if self.status(REAP).is_some() {
            return;
        }
        if let Some(pid) = self.child.process_id() {
            // SAFETY: the pid names this process's own child and nothing has
            // reaped it yet.
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        }
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::Answering;

    /// What `Answering` replies to `query`, asked from row 3 column 5.
    fn reply(query: &[u8]) -> String {
        let mut parser = vt100::Parser::new_with_callbacks(10, 20, 0, Answering::default());
        parser.process(b"\x1b[3;5H");
        parser.process(query);
        String::from_utf8(std::mem::take(&mut parser.callbacks_mut().0)).expect("utf-8")
    }

    #[test]
    fn a_cursor_position_report_answers_one_based() {
        assert_eq!(reply(b"\x1b[6n"), "\x1b[3;5R");
    }

    #[test]
    fn a_dec_private_request_is_not_that_report() {
        assert_eq!(reply(b"\x1b[?6n"), "");
    }

    #[test]
    fn no_other_status_request_is_answered() {
        assert_eq!(reply(b"\x1b[5n"), "");
        assert_eq!(reply(b"\x1b[n"), "");
    }
}
