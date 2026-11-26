// SPDX-License-Identifier: MIT OR Apache-2.0
//! A pull request file's patch, line by line, with the numbers a review comment needs.

/// What a line of a patch is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    /// `@@ -a,b +c,d @@ …`
    Hunk,
    Added,
    Removed,
    Context,
    /// `\ No newline at end of file` and anything else that is not a line of the file.
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub text: String,
    pub kind: DiffKind,
    /// The line number in the old file, for removed and unchanged lines.
    pub old: Option<u64>,
    /// The line number in the new file, for added and unchanged lines.
    pub new: Option<u64>,
}

impl DiffLine {
    /// Where a review comment on this line goes: the line number and whether it is on the old
    /// side. Hunk headers and notes cannot be commented on.
    pub fn place(&self) -> Option<(u64, bool)> {
        match self.kind {
            DiffKind::Removed => self.old.map(|n| (n, true)),
            DiffKind::Added | DiffKind::Context => self.new.map(|n| (n, false)),
            DiffKind::Hunk | DiffKind::Note => None,
        }
    }
}

/// Reads a patch as GitHub returns it (hunks only, no file headers).
pub fn diff_lines(patch: &str) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let (mut old, mut new) = (0u64, 0u64);
    for raw in patch.lines() {
        let (kind, o, n) = if let Some(rest) = raw.strip_prefix("@@ ") {
            if let Some((a, b)) = hunk_starts(rest) {
                old = a;
                new = b;
            }
            (DiffKind::Hunk, None, None)
        } else if raw.starts_with('\\') {
            (DiffKind::Note, None, None)
        } else if raw.starts_with('+') {
            let n = new;
            new = new.saturating_add(1);
            (DiffKind::Added, None, Some(n))
        } else if raw.starts_with('-') {
            let o = old;
            old = old.saturating_add(1);
            (DiffKind::Removed, Some(o), None)
        } else {
            let (o, n) = (old, new);
            old = old.saturating_add(1);
            new = new.saturating_add(1);
            (DiffKind::Context, Some(o), Some(n))
        };
        out.push(DiffLine {
            text: raw.to_owned(),
            kind,
            old: o,
            new: n,
        });
    }
    out
}

/// `-12,3 +14,5 @@ fn name` → (12, 14).
fn hunk_starts(rest: &str) -> Option<(u64, u64)> {
    let mut parts = rest.split_whitespace();
    let a = parts.next()?.strip_prefix('-')?;
    let b = parts.next()?.strip_prefix('+')?;
    let first = |s: &str| s.split(',').next().and_then(|n| n.parse::<u64>().ok());
    Some((first(a)?, first(b)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "@@ -3,4 +3,5 @@ fn main() {\n a\n-b\n+B\n+c\n d\n\\ No newline at end of file\n@@ -20 +21 @@\n-x\n+y";

    #[test]
    fn numbers_follow_the_hunks() {
        let lines = diff_lines(PATCH);
        let at = |i: usize| lines.get(i).cloned().unwrap_or_else(|| lines[0].clone());
        assert_eq!(at(0).kind, DiffKind::Hunk);
        assert_eq!((at(1).old, at(1).new), (Some(3), Some(3)));
        assert_eq!((at(2).old, at(2).new), (Some(4), None));
        assert_eq!((at(3).old, at(3).new), (None, Some(4)));
        assert_eq!((at(4).old, at(4).new), (None, Some(5)));
        assert_eq!((at(5).old, at(5).new), (Some(5), Some(6)));
        assert_eq!(at(6).kind, DiffKind::Note);
        assert_eq!(at(8).old, Some(20));
        assert_eq!(at(9).new, Some(21));
    }

    #[test]
    fn only_file_lines_take_comments() {
        let lines = diff_lines(PATCH);
        let places: Vec<_> = lines.iter().map(DiffLine::place).collect();
        assert_eq!(places[0], None);
        assert_eq!(places[2], Some((4, true)));
        assert_eq!(places[3], Some((4, false)));
        assert_eq!(places[6], None);
    }

    #[test]
    fn nonsense_does_not_panic() {
        assert!(diff_lines("").is_empty());
        let junk = diff_lines("@@ garbage\n+x\n-y\n z");
        assert_eq!(junk.len(), 4);
        let huge = diff_lines("@@ -18446744073709551615 +18446744073709551615 @@\n+a\n+b\n c");
        assert_eq!(huge.len(), 4);
    }
}
