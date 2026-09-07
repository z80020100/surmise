//! surmise — completion for `cd` directories and Git subcommands.
//!
//! `pick` is the front end. `src/main.rs` reads the arguments and hands it the
//! line a shell widget typed. Everything else here is what `pick` is built
//! from. `fixture` is the exception and belongs to the tests.
//!
//! Every module lives here rather than under the binary. `CLAUDE.md` gives the
//! two reasons.

pub mod app;
pub mod candidates;
pub mod fixture;
pub mod fuzzy;
pub mod git;
pub mod history;
pub mod keys;
pub mod line;
pub mod path;
pub mod pick;
pub mod shellword;
pub mod tty;
pub mod ui;
