//! The input buffer.
//!
//! This module owns the whole line and the cursor inside it. Text to the right
//! of the cursor therefore survives an accept and a multi-byte character never
//! splits.

#[derive(Default)]
pub struct Line {
    buf: String,
    cursor: usize, // a byte offset that is always on a char boundary
}

impl Line {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> &str {
        &self.buf
    }

    pub fn left_of_cursor(&self) -> &str {
        &self.buf[..self.cursor]
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn at_end(&self) -> bool {
        self.cursor == self.buf.len()
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.cursor = 0;
    }

    pub fn insert(&mut self, s: &str) {
        self.buf.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    /// Replace the byte range [start, cursor) and put the cursor after it.
    ///
    /// `start` is clamped to the cursor and floored to a character boundary. A
    /// caller that computes an offset from the line therefore cannot make this
    /// panic. A panic here would leave a terminal in raw mode.
    pub fn replace_back_to(&mut self, start: usize, s: &str) {
        let start = self.buf.floor_char_boundary(start.min(self.cursor));
        self.buf.replace_range(start..self.cursor, s);
        self.cursor = start + s.len();
    }

    fn prev_boundary(&self, at: usize) -> usize {
        self.buf.floor_char_boundary(at.saturating_sub(1))
    }

    fn next_boundary(&self, at: usize) -> usize {
        self.buf.ceil_char_boundary((at + 1).min(self.buf.len()))
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let p = self.prev_boundary(self.cursor);
        self.buf.replace_range(p..self.cursor, "");
        self.cursor = p;
    }

    pub fn delete(&mut self) {
        if self.at_end() {
            return;
        }
        let n = self.next_boundary(self.cursor);
        self.buf.replace_range(self.cursor..n, "");
    }

    pub fn left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.prev_boundary(self.cursor);
        }
    }

    pub fn right(&mut self) {
        if !self.at_end() {
            self.cursor = self.next_boundary(self.cursor);
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.buf.len();
    }

    pub fn kill_to_end(&mut self) {
        self.buf.truncate(self.cursor);
    }

    pub fn kill_to_start(&mut self) {
        self.buf.replace_range(..self.cursor, "");
        self.cursor = 0;
    }

    /// Walk back from `at` while the character behind it satisfies `skip`.
    fn skip_back(&self, mut at: usize, skip: impl Fn(char) -> bool) -> usize {
        while at > 0 {
            let p = self.prev_boundary(at);
            if !self.buf[p..at].chars().all(&skip) {
                break;
            }
            at = p;
        }
        at
    }

    /// Delete the word behind the cursor. A path separator ends a word. One
    /// press therefore takes back one path segment rather than the whole path.
    /// The separator itself goes with the next press. That is what keeps a
    /// repeated press from stalling on it.
    pub fn kill_word_back(&mut self) {
        let at = self.skip_back(self.cursor, char::is_whitespace);
        let at = self.skip_back(at, |c| c == '/');
        let at = self.skip_back(at, |c| !c.is_whitespace() && c != '/');
        self.buf.replace_range(at..self.cursor, "");
        self.cursor = at;
    }
}

#[cfg(test)]
mod tests {
    use super::Line;

    fn line(text: &str) -> Line {
        let mut l = Line::new();
        l.insert(text);
        l
    }

    fn with_cursor_at(text: &str, chars_in: usize) -> Line {
        let mut l = line(text);
        l.home();
        for _ in 0..chars_in {
            l.right();
        }
        l
    }

    #[test]
    fn insert_leaves_the_cursor_after_the_text() {
        let l = line("cd work");
        assert_eq!(l.text(), "cd work");
        assert_eq!(l.left_of_cursor(), "cd work");
        assert!(l.at_end());
    }

    #[test]
    fn insert_lands_at_the_cursor_rather_than_the_end() {
        let mut l = with_cursor_at("cd work", 0);
        l.insert("  ");
        assert_eq!(l.text(), "  cd work");
        assert_eq!(l.left_of_cursor(), "  ");
    }

    #[test]
    fn backspace_takes_a_whole_character_rather_than_one_byte() {
        let mut l = line("cd 好");
        l.backspace();
        assert_eq!(l.text(), "cd ");
        assert!(l.at_end());
    }

    #[test]
    fn backspace_at_the_start_does_nothing() {
        let mut l = with_cursor_at("cd", 0);
        l.backspace();
        assert_eq!(l.text(), "cd");
        assert_eq!(l.left_of_cursor(), "");
    }

    #[test]
    fn delete_takes_the_character_the_cursor_sits_on() {
        let mut l = with_cursor_at("好work", 0);
        l.delete();
        assert_eq!(l.text(), "work");
        assert_eq!(l.left_of_cursor(), "");
    }

    #[test]
    fn delete_at_the_end_does_nothing() {
        let mut l = line("cd");
        l.delete();
        assert_eq!(l.text(), "cd");
    }

    #[test]
    fn the_arrows_step_over_a_whole_character() {
        let mut l = with_cursor_at("好好", 0);
        l.right();
        assert_eq!(l.left_of_cursor(), "好");
        l.left();
        assert_eq!(l.left_of_cursor(), "");
    }

    #[test]
    fn the_arrows_stop_at_both_ends() {
        let mut l = with_cursor_at("cd", 0);
        l.left();
        assert_eq!(l.left_of_cursor(), "");
        l.end();
        l.right();
        assert!(l.at_end());
        assert_eq!(l.text(), "cd");
    }

    #[test]
    fn kill_to_end_keeps_what_is_behind_the_cursor() {
        let mut l = with_cursor_at("cd work", 2);
        l.kill_to_end();
        assert_eq!(l.text(), "cd");
        assert!(l.at_end());
    }

    #[test]
    fn kill_to_start_keeps_what_is_ahead_of_the_cursor() {
        let mut l = with_cursor_at("cd work", 3);
        l.kill_to_start();
        assert_eq!(l.text(), "work");
        assert_eq!(l.left_of_cursor(), "");
    }

    #[test]
    fn kill_word_back_takes_one_path_segment() {
        let mut l = line("cd work/alpha");
        l.kill_word_back();
        assert_eq!(l.text(), "cd work/");
    }

    #[test]
    fn a_repeated_kill_word_back_does_not_stall_on_the_separator() {
        let mut l = line("cd work/alpha");
        l.kill_word_back();
        l.kill_word_back();
        assert_eq!(l.text(), "cd ");
    }

    #[test]
    fn kill_word_back_clears_a_line_that_ends_in_a_separator() {
        let mut l = line("cd work/");
        l.kill_word_back();
        assert_eq!(l.text(), "cd ");
    }

    #[test]
    fn kill_word_back_skips_the_trailing_space_first() {
        let mut l = line("cd work   ");
        l.kill_word_back();
        assert_eq!(l.text(), "cd ");
    }

    #[test]
    fn kill_word_back_at_the_start_does_nothing() {
        let mut l = with_cursor_at("cd work", 0);
        l.kill_word_back();
        assert_eq!(l.text(), "cd work");
    }

    #[test]
    fn replace_back_to_swaps_the_token_and_keeps_the_tail() {
        let mut l = with_cursor_at("cd wo tail", 5);
        assert_eq!(l.left_of_cursor(), "cd wo");
        l.replace_back_to(3, "work/");
        assert_eq!(l.text(), "cd work/ tail");
        assert_eq!(l.left_of_cursor(), "cd work/");
    }

    #[test]
    fn replace_back_to_takes_a_multi_byte_replacement() {
        let mut l = line("cd wo");
        l.replace_back_to(3, "好/");
        assert_eq!(l.text(), "cd 好/");
        assert!(l.at_end());
        l.backspace();
        assert_eq!(l.text(), "cd 好");
    }

    #[test]
    fn replace_back_to_floors_a_start_inside_a_character() {
        let mut l = line("好work");
        // Byte 1 sits inside the leading character.
        l.replace_back_to(1, "x");
        assert_eq!(l.text(), "x");
        assert!(l.at_end());
    }

    #[test]
    fn replace_back_to_clamps_a_start_past_the_cursor() {
        let mut l = with_cursor_at("cd", 0);
        l.replace_back_to(2, "x");
        assert_eq!(l.text(), "xcd");
        assert_eq!(l.left_of_cursor(), "x");
    }

    #[test]
    fn clear_empties_both_the_line_and_the_cursor() {
        let mut l = line("cd work");
        l.clear();
        assert!(l.is_empty());
        assert!(l.at_end());
        assert_eq!(l.left_of_cursor(), "");
    }
}
