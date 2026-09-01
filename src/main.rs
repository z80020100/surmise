//! surmise — completion for the directory argument of a `cd`.
//!
//! `surmise::pick` draws the menu and answers a shell widget with the line and
//! a status. This file reads the arguments and hands the line over.

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;
use surmise::{pick, ui};

const USAGE: &str = "\
surmise — complete the directory argument of a `cd`.

  surmise --pick LINE  the picker a shell widget calls, result on stdout
";

/// What the arguments ask for.
enum Mode<'a> {
    /// The picker, over the line the widget handed over.
    Pick(&'a str),
    /// The usage.
    Help,
    /// An argument this build does not know.
    Unknown(&'a str),
}

fn mode(args: &[String]) -> Mode<'_> {
    match args.first().map(String::as_str) {
        // A widget with nothing to hand over still means the picker. An empty
        // line offers nothing and the picker answers `PASS` to it.
        Some("--pick") => Mode::Pick(args.get(1).map_or("", String::as_str)),
        // Without an argument there is no line to complete. The usage is the
        // whole of what this build can say on its own.
        None | Some("--help" | "-h") => Mode::Help,
        Some(other) => Mode::Unknown(other),
    }
}

/// One of the picker's four statuses as the process's own.
fn status(code: i32) -> ExitCode {
    // The contract's four statuses are 0 to 3 and a status is a byte anyway.
    ExitCode::from(code as u8)
}

/// Put the usage on `out`. A reader that stops early is not worth a panic and
/// `surmise --help | head` is an ordinary thing to type.
fn usage(mut out: impl Write) {
    let _ = out.write_all(USAGE.as_bytes());
}

/// The picker, with the status its contract asks for.
///
/// An I/O failure answers `PASS` rather than a failure status, because the
/// widget reads `1` as `CANCELLED` and would take the person's line away
/// without a word. `PASS` hands the key back to the shell instead.
///
/// The message goes with it. The widget owns the screen and a line on stderr
/// would land in the middle of what it is drawing.
fn picker(seed: &str) -> ExitCode {
    status(pick::run(seed).unwrap_or(pick::PASS))
}

fn main() -> ExitCode {
    // `args` panics on an argument that is not UTF-8 and a shell line can hold
    // one. surmise cannot complete what it cannot read and `PASS` hands the key
    // back. A panic would put its own report on the row the widget is drawing.
    let read: Result<Vec<String>, _> = std::env::args_os()
        .skip(1)
        .map(OsString::into_string)
        .collect();
    let Ok(args) = read else {
        return status(pick::PASS);
    };

    match mode(&args) {
        Mode::Pick(seed) => picker(seed),
        Mode::Help => {
            usage(io::stdout());
            ExitCode::SUCCESS
        }
        Mode::Unknown(arg) => {
            // An argument reaches the terminal here. A directory name reaches
            // it in the frame and both go through the same filter.
            eprintln!("surmise: unexpected argument: {}", ui::printable(arg));
            usage(io::stderr());
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An argument list as `main` collects it.
    fn args(list: &[&str]) -> Vec<String> {
        list.iter().copied().map(String::from).collect()
    }

    #[test]
    fn no_argument_at_all_asks_for_the_usage() {
        assert!(matches!(mode(&args(&[])), Mode::Help));
    }

    #[test]
    fn pick_takes_the_line_that_follows_it() {
        let a = args(&["--pick", "cd wo"]);
        assert!(matches!(mode(&a), Mode::Pick("cd wo")));
    }

    #[test]
    fn pick_with_no_line_is_still_the_picker() {
        // The widget quotes the line it hands over. An empty one therefore
        // arrives as an empty argument rather than as no argument at all.
        assert!(matches!(mode(&args(&["--pick", ""])), Mode::Pick("")));
        assert!(matches!(mode(&args(&["--pick"])), Mode::Pick("")));
    }

    #[test]
    fn a_line_that_looks_like_an_option_is_still_the_line() {
        assert!(matches!(mode(&args(&["--pick", "-h"])), Mode::Pick("-h")));
    }

    #[test]
    fn both_spellings_ask_for_the_usage() {
        assert!(matches!(mode(&args(&["--help"])), Mode::Help));
        assert!(matches!(mode(&args(&["-h"])), Mode::Help));
    }

    #[test]
    fn an_argument_this_build_does_not_know_is_never_the_picker() {
        assert!(matches!(mode(&args(&["--nope"])), Mode::Unknown("--nope")));
        assert!(matches!(mode(&args(&["cd wo"])), Mode::Unknown("cd wo")));
    }

    /// `status` casts to a byte and this is the claim that makes it lossless.
    #[test]
    fn every_contract_status_fits_an_exit_code() {
        for code in [pick::ACCEPTED, pick::CANCELLED, pick::PASS, pick::RUN] {
            assert!(u8::try_from(code).is_ok());
        }
    }
}
