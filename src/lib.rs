//! surmise.
//!
//! What this crate does is TBD. The modules are landing one at a time and the
//! binary is still a placeholder.
//!
//! Every module lives here rather than under the binary. `CLAUDE.md` gives the
//! two reasons.

pub mod app;
pub mod candidates;
#[cfg(test)]
mod fixture;
pub mod fuzzy;
pub mod line;
pub mod path;
pub mod shellword;
