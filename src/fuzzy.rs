//! Subsequence matcher with positional scoring.
//!
//! Every needle character has to appear in order. Where it appears decides the
//! score. A run of adjacent matches scores highest, then a match at the start,
//! then a match just after a separator. A long haystack pays a small penalty.

/// Case-fold one character. Folding per character rather than per string keeps
/// the result one character wide. An index into the folded text is therefore
/// also an index into the original. `char::to_lowercase` does not promise
/// that: `İ` lowercases to two characters.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// How many bytes of `s` the two share from the front with their case set
/// aside. The count lands on a character boundary of `s`.
///
/// The count is in `s`'s own bytes and the pair is therefore not
/// interchangeable: a folded character can be a different width in each of
/// them. `shared_bytes("İ", "i")` is 2 and `shared_bytes("i", "İ")` is 1.
pub(crate) fn shared_bytes(s: &str, other: &str) -> usize {
    s.chars()
        .zip(other.chars())
        .take_while(|(a, b)| fold(*a) == fold(*b))
        .map(|(a, _)| a.len_utf8())
        .sum()
}

/// Does `s` start with `prefix` once the case of each is set aside? A person
/// types a name in whichever case is to hand and the directory keeps its own.
pub(crate) fn starts_with_folded(s: &str, prefix: &str) -> bool {
    shared_bytes(prefix, s) == prefix.len()
}

/// Where each character of `n` lands in `h`, in order. `None` when it is not
/// a subsequence. The characters of `n` are folded already and those of `h`
/// are folded here.
fn positions(n: &[char], h: &[char]) -> Option<Vec<usize>> {
    let mut at = 0usize;
    let mut out = Vec::with_capacity(n.len());
    for &want in n {
        let idx = (at..h.len()).find(|&i| fold(h[i]) == want)?;
        at = idx + 1;
        out.push(idx);
    }
    Some(out)
}

/// Where what was typed landed in `haystack`, as character indices. Empty
/// when it is not a subsequence and empty for an empty needle. The menu marks
/// these characters and it is the same walk the score is measured along.
pub(crate) fn matched(needle: &str, haystack: &str) -> Vec<usize> {
    let n: Vec<char> = needle.chars().map(fold).collect();
    let h: Vec<char> = haystack.chars().collect();
    positions(&n, &h).unwrap_or_default()
}

/// Score `needle` against `haystack`. None means it is not a subsequence. A
/// score can go negative when the match is late in a long haystack.
pub fn score(needle: &str, haystack: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    let n: Vec<char> = needle.chars().map(fold).collect();
    let h: Vec<char> = haystack.chars().collect();

    let mut total = 0i32;
    let mut prev: Option<usize> = None;

    for idx in positions(&n, &h)? {
        let mut points = 10;
        match prev {
            Some(p) if idx == p + 1 => points += 16,
            Some(p) => points -= ((idx - p - 1) as i32).min(12),
            None if idx == 0 => points += 24,
            None => points -= (idx as i32).min(12),
        }
        if idx > 0 && matches!(h[idx - 1], '/' | '-' | '_' | '.' | ' ') {
            points += 14;
        }
        total += points;
        prev = Some(idx);
    }
    total -= (h.len().saturating_sub(n.len()) as i32) / 4;
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::{matched, score, shared_bytes, starts_with_folded};

    #[test]
    fn an_empty_needle_matches_everything() {
        assert_eq!(score("", "work"), Some(0));
        assert_eq!(score("", ""), Some(0));
    }

    #[test]
    fn a_character_that_is_absent_does_not_match() {
        assert_eq!(score("z", "work"), None);
        assert_eq!(score("work", "wor"), None);
    }

    #[test]
    fn the_order_has_to_hold() {
        assert!(score("wk", "work").is_some());
        assert_eq!(score("kw", "work"), None);
    }

    #[test]
    fn adjacent_beats_scattered() {
        let together = score("wo", "work").unwrap();
        let apart = score("wo", "wxork").unwrap();
        assert!(together > apart, "{together} should beat {apart}");
    }

    #[test]
    fn the_start_beats_the_middle() {
        let start = score("w", "work").unwrap();
        let middle = score("w", "awork").unwrap();
        assert!(start > middle, "{start} should beat {middle}");
    }

    #[test]
    fn a_separator_lifts_what_follows_it() {
        for sep in ['/', '-', '_', '.', ' '] {
            let after = score("w", &format!("a{sep}work")).unwrap();
            let inside = score("w", "aawork").unwrap();
            assert!(after > inside, "{sep:?}: {after} should beat {inside}");
        }
    }

    #[test]
    fn a_shorter_haystack_beats_a_longer_one() {
        let short = score("w", "work").unwrap();
        let long = score("w", "workworkworkwork").unwrap();
        assert!(short > long, "{short} should beat {long}");
    }

    #[test]
    fn the_case_does_not_matter() {
        assert_eq!(score("WORK", "work"), score("work", "work"));
        assert_eq!(score("work", "WORK"), score("work", "work"));
        assert!(score("w", "Work").is_some());
    }

    #[test]
    fn a_character_whose_lower_case_is_wider_does_not_panic() {
        // `İ` lowercases to two characters. Indexing the original text with an
        // index into a lowercased copy used to read past the end.
        assert_eq!(score("\u{307}\u{307}", "İİ"), None);
        assert!(score("i", "İ").is_some());
        assert!(score("İ", "i").is_some());
    }

    #[test]
    fn a_late_match_in_a_long_haystack_scores_below_zero() {
        // The caller ranks on this number and does not filter it. The sign is
        // therefore part of the contract rather than an accident.
        let late = score("z", &format!("{}z", "a".repeat(50))).unwrap();
        assert!(late < 0, "{late} should be negative");
    }

    #[test]
    fn a_multi_byte_haystack_scores_by_character() {
        assert!(score("好", "好work").is_some());
        assert!(score("w", "好work").is_some());
        assert_eq!(score("好", "work"), None);
    }

    #[test]
    fn a_folded_prefix_ignores_the_case_and_counts_whole_characters() {
        assert!(starts_with_folded("Work/", "wo"));
        assert!(!starts_with_folded("work/", "wk"));
        assert!(!starts_with_folded("wo", "work/"));
        assert_eq!(shared_bytes("日本語", "日本"), 6);
        assert_eq!(shared_bytes("Work/", "worse/"), 3);
    }

    #[test]
    fn a_match_says_where_each_character_landed() {
        assert_eq!(matched("wk", "work"), [0, 3]);
        assert_eq!(matched("work", "work"), [0, 1, 2, 3]);
        assert_eq!(matched("W", "work"), [0]);
    }

    #[test]
    fn nothing_typed_and_nothing_matched_both_mark_nothing() {
        assert!(matched("", "work").is_empty());
        assert!(matched("z", "work").is_empty());
    }

    #[test]
    fn a_position_counts_characters_rather_than_bytes() {
        assert_eq!(matched("w", "\u{65e5}\u{672c}work"), [2]);
    }

    #[test]
    fn a_match_takes_the_first_character_that_will_serve() {
        // The walk is greedy and the score is measured along this same one. A
        // mark anywhere else would say the row scored other than it did.
        assert_eq!(matched("o", "work"), [1]);
        assert_eq!(matched("ok", "oook"), [0, 3]);
    }

    #[test]
    fn a_shared_count_is_in_the_first_arguments_own_bytes() {
        // The pair is not interchangeable. A caller that swaps them gets an
        // index into the wrong string and the compiler cannot say so.
        assert_eq!(shared_bytes("İ", "i"), 2);
        assert_eq!(shared_bytes("i", "İ"), 1);
    }
}
