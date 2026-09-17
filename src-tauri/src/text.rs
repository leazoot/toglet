//! What a piece of text may look like once it leaves Rust.
//!
//! One rule, one place: session excerpts (`app_server::thread`) and the third-party sentences a
//! reset feed carries (`resets`) both pass through here. Two copies of a truncation rule drift,
//! and the one that drifts is the one nobody is looking at.

/// One line, whitespace folded, cut to `cap` characters with an ellipsis; `None` when nothing
/// is left.
pub fn one_line(text: &str, cap: usize) -> Option<String> {
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
        if line.chars().count() > cap {
            break;
        }
    }
    if line.is_empty() {
        return None;
    }
    if line.chars().count() > cap {
        let mut cut: String = line.chars().take(cap).collect();
        cut.push('…');
        return Some(cut);
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_is_folded_onto_one_line() {
        assert_eq!(
            one_line("  Reset\n\nall   propagated.\tSweet dreams. ", 100),
            Some("Reset all propagated. Sweet dreams.".to_owned())
        );
    }

    #[test]
    fn a_long_text_is_cut_at_the_cap_with_an_ellipsis() {
        let cut = one_line(&"word ".repeat(80), 20).expect("something is left");
        assert_eq!(cut.chars().count(), 21);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn a_blank_text_is_none_not_an_empty_string() {
        assert_eq!(one_line("   \n\t ", 10), None);
        assert_eq!(one_line("", 10), None);
    }

    #[test]
    fn the_cap_counts_characters_not_bytes() {
        assert_eq!(one_line("重置已全部生效", 4), Some("重置已全…".to_owned()));
    }
}
