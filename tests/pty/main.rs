//! surmise on a real terminal.
//!
//! Every pty test lives in this one binary. A second binary would compile
//! `term` again and `dead_code` would turn the gate red. `CLAUDE.md` gives
//! the whole of it.

mod pick;
mod term;
mod zsh;
