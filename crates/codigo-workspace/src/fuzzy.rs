// SPDX-License-Identifier: MIT OR Apache-2.0
//! Fuzzy matching for quick open and the command palette.
//!
//! A candidate matches when it contains the query's characters in order. The score favours
//! matches that start words (after `/`, `_`, `-`, `.`, a space, or a lower-to-upper step), that
//! run together, that fall in the file name rather than the folders, and that begin early. The
//! best alignment is found with dynamic programming, so `mrs` finds `src/main.rs` through its
//! `m` and `rs` and not through whatever letters happen to come first.

const MATCH: i32 = 16;
const CONSECUTIVE: i32 = 10;
const GAP: i32 = 2;
const LEADING: i32 = 1;
const LEADING_CAP: i32 = 12;
const BOUNDARY: i32 = 14;
const CAMEL: i32 = 10;
const FIRST: i32 = 12;
const IN_NAME: i32 = 6;
const EXACT_CASE: i32 = 1;
/// A candidate this long is not a path or a command; do not spend time on it.
const MAX_CANDIDATE_CHARS: usize = 1024;
const NONE: i32 = i32::MIN / 2;

/// Matches one query against many candidates, reusing its buffers.
pub struct Scorer {
    query: Vec<char>,
    lower: Vec<char>,
    best: Vec<i32>,
    carry: Vec<i32>,
    bonus: Vec<i32>,
    chars: Vec<char>,
    cand_lower: Vec<char>,
}

impl Scorer {
    pub fn new(query: &str) -> Self {
        let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
        let lower = query.iter().map(|c| fold(*c)).collect();
        Self {
            query,
            lower,
            best: Vec::new(),
            carry: Vec::new(),
            bonus: Vec::new(),
            chars: Vec::new(),
            cand_lower: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.query.is_empty()
    }

    /// The score of `candidate`, or `None` when it does not match. Higher is better.
    pub fn score(&mut self, candidate: &str) -> Option<i32> {
        self.run(candidate).map(|(score, _)| score)
    }

    /// The score and the character indices (not bytes) that matched, for highlighting.
    pub fn matches(&mut self, candidate: &str) -> Option<(i32, Vec<usize>)> {
        let (score, last) = self.run(candidate)?;
        Some((score, self.backtrack(last)))
    }

    fn run(&mut self, candidate: &str) -> Option<(i32, usize)> {
        let m = self.query.len();
        if m == 0 {
            return Some((0, 0));
        }
        self.chars.clear();
        self.chars.extend(candidate.chars());
        let n = self.chars.len();
        if n < m || n > MAX_CANDIDATE_CHARS {
            return None;
        }
        self.cand_lower.clear();
        self.cand_lower.extend(self.chars.iter().map(|c| fold(*c)));
        if !is_subsequence(&self.lower, &self.cand_lower) {
            return None;
        }
        self.compute_bonus();

        self.best.clear();
        self.best.resize(m * n, NONE);
        self.carry.clear();
        self.carry.resize(m * n, NONE);
        for i in 0..m {
            for j in 0..n {
                let at = i * n + j;
                if self.lower.get(i) == self.cand_lower.get(j) {
                    let base = MATCH
                        + self.bonus.get(j).copied().unwrap_or(0)
                        + i32::from(self.query.get(i) == self.chars.get(j)) * EXACT_CASE;
                    let before = if i == 0 {
                        -(j as i32).min(LEADING_CAP) * LEADING
                    } else if j == 0 {
                        NONE
                    } else {
                        let prev = (i - 1) * n + (j - 1);
                        let run = self.best.get(prev).map_or(NONE, |b| b + CONSECUTIVE);
                        let gapped = self.carry.get(prev).copied().unwrap_or(NONE);
                        run.max(gapped)
                    };
                    if before > NONE / 2 {
                        if let Some(cell) = self.best.get_mut(at) {
                            *cell = base + before;
                        }
                    }
                }
                let here = self.best.get(at).copied().unwrap_or(NONE);
                let left = if j == 0 {
                    NONE
                } else {
                    self.carry.get(at - 1).map_or(NONE, |c| c - GAP)
                };
                if let Some(cell) = self.carry.get_mut(at) {
                    *cell = here.max(left);
                }
            }
        }
        let row = (m - 1) * n;
        let (last, score) = (0..n)
            .filter_map(|j| self.best.get(row + j).map(|s| (j, *s)))
            .filter(|(_, s)| *s > NONE / 2)
            .max_by_key(|(j, s)| (*s, std::cmp::Reverse(*j)))?;
        Some((score, last))
    }

    fn backtrack(&self, last: usize) -> Vec<usize> {
        let m = self.query.len();
        let n = self.chars.len();
        let mut out = vec![0; m];
        let mut j = last;
        for i in (0..m).rev() {
            if let Some(slot) = out.get_mut(i) {
                *slot = j;
            }
            if i == 0 {
                break;
            }
            let here = self.best.get(i * n + j).copied().unwrap_or(NONE);
            let base = MATCH
                + self.bonus.get(j).copied().unwrap_or(0)
                + i32::from(self.query.get(i) == self.chars.get(j)) * EXACT_CASE;
            let prev_row = (i - 1) * n;
            // Consecutive with the previous query character, if that explains the score.
            let consecutive = j
                .checked_sub(1)
                .and_then(|p| self.best.get(prev_row + p))
                .map(|b| b + CONSECUTIVE + base);
            if consecutive == Some(here) {
                j -= 1;
                continue;
            }
            // Otherwise the best earlier position, paying the gap.
            let mut pick = j.saturating_sub(1);
            let mut top = NONE;
            for k in 0..j {
                let score = self.best.get(prev_row + k).copied().unwrap_or(NONE);
                if score <= NONE / 2 {
                    continue;
                }
                let gapped = score - ((j - 1 - k) as i32) * GAP;
                if gapped >= top {
                    top = gapped;
                    pick = k;
                }
            }
            j = pick;
        }
        out
    }

    fn compute_bonus(&mut self) {
        self.bonus.clear();
        let name_start = self
            .chars
            .iter()
            .rposition(|c| *c == '/' || *c == '\\')
            .map_or(0, |p| p + 1);
        let mut prev: Option<char> = None;
        for (j, c) in self.chars.iter().enumerate() {
            let mut b = match prev {
                None => FIRST,
                Some(p) if is_separator(p) => BOUNDARY,
                Some(p) if p.is_lowercase() && c.is_uppercase() => CAMEL,
                Some(p) if p.is_alphabetic() != c.is_alphabetic() => CAMEL / 2,
                _ => 0,
            };
            if j >= name_start && name_start > 0 {
                b += IN_NAME;
            }
            self.bonus.push(b);
            prev = Some(*c);
        }
    }
}

fn is_separator(c: char) -> bool {
    matches!(c, '/' | '\\' | '_' | '-' | '.' | ' ' | ':')
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn is_subsequence(query: &[char], candidate: &[char]) -> bool {
    let mut q = query.iter();
    let mut want = q.next();
    for c in candidate {
        if Some(c) == want {
            want = q.next();
        }
    }
    want.is_none()
}

/// One ranked candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ranked {
    /// Position in the input slice.
    pub index: usize,
    pub score: i32,
    /// Character indices that matched.
    pub positions: Vec<usize>,
}

/// The best `limit` candidates for `query`, best first; ties keep input order. An empty query
/// returns the first `limit` candidates unscored.
pub fn rank<T>(query: &str, items: &[T], key: impl Fn(&T) -> &str, limit: usize) -> Vec<Ranked> {
    let mut scorer = Scorer::new(query);
    if scorer.is_empty() {
        return (0..items.len().min(limit))
            .map(|index| Ranked {
                index,
                score: 0,
                positions: Vec::new(),
            })
            .collect();
    }
    let mut scored: Vec<(usize, i32)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| scorer.score(key(item)).map(|s| (i, s)))
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.truncate(limit);
    scored
        .into_iter()
        .map(|(index, score)| Ranked {
            index,
            score,
            positions: items
                .get(index)
                .and_then(|item| scorer.matches(key(item)))
                .map(|(_, p)| p)
                .unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(query: &str, items: &[&str]) -> Vec<String> {
        rank(query, items, |s| s, 10)
            .into_iter()
            .filter_map(|r| items.get(r.index).map(|s| s.to_string()))
            .collect()
    }

    #[test]
    fn needs_the_characters_in_order() {
        let mut s = Scorer::new("abc");
        assert!(s.score("a-b-c").is_some());
        assert!(s.score("acb").is_none());
        assert!(s.score("ab").is_none());
        assert!(Scorer::new("").score("anything").is_some());
    }

    #[test]
    fn ignores_case_and_spaces_in_the_query() {
        let mut s = Scorer::new("MA in");
        assert!(s.score("src/main.rs").is_some());
    }

    #[test]
    fn file_names_beat_folders_and_word_starts_beat_the_middle() {
        let items = ["domain.rs", "src/main.rs", "docs/maintenance/notes.md"];
        assert_eq!(top("main", &items)[0], "src/main.rs");
        let items = ["xxsettingsxx", "user_settings", "settings"];
        assert_eq!(top("settings", &items)[0], "settings");
        assert_eq!(top("us", &["bus", "user"])[0], "user");
    }

    #[test]
    fn camel_case_and_initials() {
        let items = ["helper.rs", "HashMapHelper.rs", "hmh_misc.txt"];
        let ranked = top("hmh", &items);
        assert!(ranked.contains(&"HashMapHelper.rs".to_string()));
        assert!(!ranked.contains(&"helper.rs".to_string()));
    }

    #[test]
    fn positions_name_the_characters_that_matched() {
        let mut s = Scorer::new("mrs");
        let (_, p) = s.matches("src/main.rs").unwrap_or_default();
        assert_eq!(p, vec![4, 9, 10]);
        let mut s = Scorer::new("main");
        let (_, p) = s.matches("src/main.rs").unwrap_or_default();
        assert_eq!(p, vec![4, 5, 6, 7]);
    }

    #[test]
    fn positions_are_character_indices_for_multibyte_text() {
        let mut s = Scorer::new("ñu");
        let (_, p) = s.matches("año_nuevo").unwrap_or_default();
        assert_eq!(p.len(), 2);
        let chars: Vec<char> = "año_nuevo".chars().collect();
        assert_eq!(chars[p[0]], 'ñ');
        assert_eq!(chars[p[1]], 'u');
    }

    #[test]
    fn ties_keep_input_order_and_limit_applies() {
        let items = ["a1", "a2", "a3", "a4"];
        let r = rank("a", &items, |s| s, 3);
        assert_eq!(r.iter().map(|r| r.index).collect::<Vec<_>>(), vec![0, 1, 2]);
        let none = rank("", &items, |s| s, 2);
        assert_eq!(none.len(), 2);
    }

    #[test]
    fn hostile_candidates_are_skipped_not_slow() {
        let mut s = Scorer::new("a");
        assert!(s.score(&"a".repeat(MAX_CANDIDATE_CHARS + 1)).is_none());
        assert!(s.score("").is_none());
        let mut long = Scorer::new(&"a".repeat(40));
        assert!(long.score(&"a".repeat(900)).is_some());
    }
}
