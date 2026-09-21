//! surmise — completion for `cd` directories, Git subcommands, and any
//! command with a committed specification.
//!
//! `pick` is the front end. `src/main.rs` reads the arguments and hands it the
//! line a shell widget typed. Everything else here is what `pick` is built
//! from. `fixture` is the exception and belongs to the tests.
//!
//! Every module lives here rather than under the binary. `CLAUDE.md` gives the
//! two reasons.

pub mod app;
pub mod argwalk;
pub mod candidates;
pub mod config;
pub mod fixture;
pub mod fuzzy;
pub mod git;
pub mod histfile;
pub mod history;
pub mod keys;
pub mod line;
pub mod native;
pub mod path;
pub mod pick;
pub mod shellparse;
pub mod shellword;
pub mod spec;
pub mod spec_menu;
pub mod spec_store;
pub mod state;
pub mod tty;
pub mod ui;
