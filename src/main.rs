//! surmise — completion for the directory argument of a `cd`.
//!
//! `surmise::pick` draws the menu and answers a shell widget with the line and
//! a status. This file reads the arguments and hands the line over. It also
//! carries the widget itself, because `cargo install` places a binary and
//! nothing else.

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;
use surmise::{pick, ui};

const USAGE: &str = "\
surmise — complete the directory argument of a `cd`.

  surmise init zsh     the shell widget, for `eval \"$(surmise init zsh)\"`
  surmise --pick LINE  the picker that widget calls, result on stdout
";

/// The zsh widget, compiled in. `cargo install` places a binary and has no
/// mechanism for anything beside it. A widget that lived only in the
/// repository would therefore never reach an installed surmise.
const ZSH: &str = include_str!("../shell/surmise.zsh");

/// What the arguments ask for.
enum Mode<'a> {
    /// The picker, over the line the widget handed over.
    Pick(&'a str),
    /// The widget for the named shell. An empty name is no name.
    Init(&'a str),
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
        // `init` with no shell names none. It stays an `Init` rather than
        // become its own mode, because a missing name is a wrong one.
        //
        // `init` names one shell and nothing else. A third argument is a typo
        // and the arm that reports an unknown one already says so. `--pick`
        // takes every argument after the first, because a shell line can look
        // like anything and the widget is the only caller it has.
        Some("init") => match args.get(2) {
            Some(extra) => Mode::Unknown(extra),
            None => Mode::Init(args.get(1).map_or("", String::as_str)),
        },
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

/// A complaint, the usage behind it, and the failure status. Every refusal
/// this build has says all three and the three belong together.
fn refuse(complaint: &str) -> ExitCode {
    eprintln!("surmise: {complaint}");
    usage(io::stderr());
    ExitCode::FAILURE
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
        // The widget goes out whole. `make shell` reads the same bytes and a
        // substitution here would put the gate on something other than what
        // ships.
        //
        // The widget is the whole of what this command delivers. A write that
        // failed therefore cannot report success. `eval "$(surmise init zsh)"`
        // would otherwise evaluate a truncated widget or nothing at all and
        // say nothing about either.
        Mode::Init("zsh") => {
            let mut out = io::stdout().lock();
            match out.write_all(ZSH.as_bytes()).and_then(|()| out.flush()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("surmise: cannot write the zsh widget: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Mode::Init("") => refuse("init takes a shell. zsh is the only one this build has."),
        Mode::Init(shell) => refuse(&format!(
            "no widget for {}. zsh is the only shell this build has.",
            ui::printable(shell)
        )),
        Mode::Help => {
            usage(io::stdout());
            ExitCode::SUCCESS
        }
        // An argument reaches the terminal here. A directory name reaches it
        // in the frame and both go through the same filter.
        Mode::Unknown(arg) => refuse(&format!("unexpected argument: {}", ui::printable(arg))),
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

    #[test]
    fn init_takes_the_shell_that_follows_it() {
        assert!(matches!(mode(&args(&["init", "zsh"])), Mode::Init("zsh")));
        assert!(matches!(mode(&args(&["init", "bash"])), Mode::Init("bash")));
    }

    #[test]
    fn a_third_argument_to_init_is_never_a_shell() {
        let a = args(&["init", "zsh", "--nope"]);
        assert!(matches!(mode(&a), Mode::Unknown("--nope")));
    }

    #[test]
    fn init_with_no_shell_is_still_init() {
        assert!(matches!(mode(&args(&["init"])), Mode::Init("")));
    }

    /// `include_str!` cannot say what it took. This is the claim that the
    /// bytes `init` prints are the widget rather than some other file.
    #[test]
    fn the_compiled_in_widget_is_the_zsh_widget() {
        assert!(ZSH.contains("zle -N surmise-complete"));
        assert!(ZSH.contains("zle -N surmise-space"));
        assert!(ZSH.contains("bindkey '^I' surmise-complete"));
    }

    /// `status` casts to a byte and this is the claim that makes it lossless.
    #[test]
    fn every_contract_status_fits_an_exit_code() {
        for code in [pick::ACCEPTED, pick::CANCELLED, pick::PASS, pick::RUN] {
            assert!(u8::try_from(code).is_ok());
        }
    }
}
