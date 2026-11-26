// SPDX-License-Identifier: MIT OR Apache-2.0
//! Blame at the end of the caret's line: who changed it, when, and why.

use codigo_git::BlameLine;

use crate::i18n::{tr, trf};

/// What is written after the line: `Ana, 3 days ago · Fix the parser`.
pub fn label(line: &BlameLine, now: i64) -> String {
    if line.uncommitted() {
        return tr("Not committed yet").to_owned();
    }
    let mut out = format!("{}, {}", line.author, age(now - line.time));
    if !line.summary.is_empty() {
        out.push_str(" · ");
        out.push_str(&line.summary);
    }
    out
}

/// A span in seconds as a person would say it.
pub fn age(seconds: i64) -> String {
    let s = seconds.max(0);
    let (n, one, many) = match s {
        0..=59 => return tr("just now").to_owned(),
        60..=3_599 => (s / 60, "1 minute ago", "{} minutes ago"),
        3_600..=86_399 => (s / 3_600, "1 hour ago", "{} hours ago"),
        86_400..=2_591_999 => (s / 86_400, "1 day ago", "{} days ago"),
        2_592_000..=31_535_999 => (s / 2_592_000, "1 month ago", "{} months ago"),
        _ => (s / 31_536_000, "1 year ago", "{} years ago"),
    };
    if n == 1 {
        tr(one).to_owned()
    } else {
        trf(many, &[&n.to_string()])
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(short: &str, author: &str, time: i64, summary: &str) -> BlameLine {
        BlameLine {
            short: short.into(),
            author: author.into(),
            time,
            summary: summary.into(),
        }
    }

    #[test]
    fn ages_pick_the_unit() {
        crate::i18n::set(crate::i18n::Lang::En);
        assert_eq!(age(5), "just now");
        assert_eq!(age(60), "1 minute ago");
        assert_eq!(age(3 * 3_600), "3 hours ago");
        assert_eq!(age(2 * 86_400), "2 days ago");
        assert_eq!(age(400 * 86_400), "1 year ago");
        assert_eq!(age(-100), "just now");
    }

    #[test]
    fn labels_name_author_age_and_summary() {
        crate::i18n::set(crate::i18n::Lang::En);
        let l = line("abc1234", "Ana", 0, "Fix the parser");
        assert_eq!(label(&l, 2 * 86_400), "Ana, 2 days ago · Fix the parser");
        assert_eq!(label(&line("0000000", "x", 0, ""), 10), "Not committed yet");
    }
}
