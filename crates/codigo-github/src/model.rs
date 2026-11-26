// SPDX-License-Identifier: MIT OR Apache-2.0
//! GitHub's JSON, read into the few fields Código shows. Every field is optional on the wire
//! as far as this code is concerned: a missing one is an empty string or zero, never a panic.

use serde_json::Value;

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn n(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn b(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn login(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|u| u.get("login"))
        .and_then(Value::as_str)
        .unwrap_or("ghost")
        .to_owned()
}

fn labels(v: &Value) -> Vec<String> {
    v.get("labels")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|l| l.get("name")?.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// `owner/name` from an API repository URL such as `https://api.github.com/repos/a/b`.
fn repo_of_url(url: &str) -> String {
    url.split("/repos/").nth(1).unwrap_or_default().to_owned()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub login: String,
    pub name: Option<String>,
    pub html_url: String,
}

impl User {
    pub fn parse(v: &Value) -> User {
        User {
            login: s(v, "login"),
            name: v
                .get("name")
                .and_then(Value::as_str)
                .filter(|x| !x.is_empty())
                .map(str::to_owned),
            html_url: s(v, "html_url"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoInfo {
    /// `owner/name`.
    pub full_name: String,
    pub description: String,
    pub private: bool,
    pub fork: bool,
    pub language: Option<String>,
    pub stars: u64,
    pub clone_url: String,
    pub html_url: String,
    pub default_branch: String,
    pub pushed_at: String,
}

impl RepoInfo {
    pub fn parse(v: &Value) -> RepoInfo {
        RepoInfo {
            full_name: s(v, "full_name"),
            description: s(v, "description"),
            private: b(v, "private"),
            fork: b(v, "fork"),
            language: v.get("language").and_then(Value::as_str).map(str::to_owned),
            stars: n(v, "stargazers_count"),
            clone_url: s(v, "clone_url"),
            html_url: s(v, "html_url"),
            default_branch: s(v, "default_branch"),
            pushed_at: s(v, "pushed_at"),
        }
    }
}

/// A pull request, from a list, a search or its own page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pull {
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
    pub draft: bool,
    pub author: String,
    pub created_at: String,
    pub updated_at: String,
    pub html_url: String,
    pub head_ref: String,
    pub head_sha: String,
    pub base_ref: String,
    pub comments: u64,
    pub labels: Vec<String>,
}

impl Pull {
    /// From `/repos/{o}/{r}/pulls` (or one of them), where `repo` is known by the caller.
    pub fn parse(v: &Value, repo: &str) -> Pull {
        let merged = b(v, "merged") || v.get("merged_at").is_some_and(|m| !m.is_null());
        let state = if merged {
            "merged".to_owned()
        } else {
            s(v, "state")
        };
        Pull {
            repo: repo.to_owned(),
            number: n(v, "number"),
            title: s(v, "title"),
            body: s(v, "body"),
            state,
            draft: b(v, "draft"),
            author: login(v, "user"),
            created_at: s(v, "created_at"),
            updated_at: s(v, "updated_at"),
            html_url: s(v, "html_url"),
            head_ref: v.get("head").map(|h| s(h, "ref")).unwrap_or_default(),
            head_sha: v.get("head").map(|h| s(h, "sha")).unwrap_or_default(),
            base_ref: v.get("base").map(|h| s(h, "ref")).unwrap_or_default(),
            comments: n(v, "comments"),
            labels: labels(v),
        }
    }

    /// From a search result, which names its repository by URL.
    pub fn parse_search(v: &Value) -> Pull {
        let repo = repo_of_url(&s(v, "repository_url"));
        let mut p = Pull::parse(v, &repo);
        if v.get("pull_request")
            .and_then(|p| p.get("merged_at"))
            .is_some_and(|m| !m.is_null())
        {
            p.state = "merged".into();
        }
        p
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullDetail {
    pub pull: Pull,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    /// GitHub's word for whether it can merge: `clean`, `dirty`, `blocked`, `unstable`…
    pub mergeable_state: String,
}

impl PullDetail {
    pub fn parse(v: &Value, repo: &str) -> PullDetail {
        PullDetail {
            pull: Pull::parse(v, repo),
            additions: n(v, "additions"),
            deletions: n(v, "deletions"),
            changed_files: n(v, "changed_files"),
            mergeable_state: s(v, "mergeable_state"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: String,
    pub author: String,
    pub updated_at: String,
    pub html_url: String,
    pub comments: u64,
    pub labels: Vec<String>,
}

impl Issue {
    pub fn parse(v: &Value, repo: &str) -> Issue {
        let repo = if repo.is_empty() {
            repo_of_url(&s(v, "repository_url"))
        } else {
            repo.to_owned()
        };
        Issue {
            repo,
            number: n(v, "number"),
            title: s(v, "title"),
            body: s(v, "body"),
            state: s(v, "state"),
            author: login(v, "user"),
            updated_at: s(v, "updated_at"),
            html_url: s(v, "html_url"),
            comments: n(v, "comments"),
            labels: labels(v),
        }
    }

    /// Whether this "issue" is really a pull request (GitHub lists both together).
    pub fn is_pull(v: &Value) -> bool {
        v.get("pull_request").is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullFile {
    pub filename: String,
    /// `added`, `modified`, `removed`, `renamed`…
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    /// The diff; absent for binary or very large files.
    pub patch: Option<String>,
}

impl PullFile {
    pub fn parse(v: &Value) -> PullFile {
        PullFile {
            filename: s(v, "filename"),
            status: s(v, "status"),
            additions: n(v, "additions"),
            deletions: n(v, "deletions"),
            patch: v.get("patch").and_then(Value::as_str).map(str::to_owned),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommentKind {
    Comment,
    /// A review, with its verdict: `APPROVED`, `CHANGES_REQUESTED`, `COMMENTED`…
    Review(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub kind: CommentKind,
}

impl Comment {
    pub fn parse_comment(v: &Value) -> Comment {
        Comment {
            author: login(v, "user"),
            body: s(v, "body"),
            created_at: s(v, "created_at"),
            kind: CommentKind::Comment,
        }
    }

    pub fn parse_review(v: &Value) -> Comment {
        Comment {
            author: login(v, "user"),
            body: s(v, "body"),
            created_at: s(v, "submitted_at"),
            kind: CommentKind::Review(s(v, "state")),
        }
    }
}

/// A check on a commit (CI).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    /// `queued`, `in_progress` or `completed`.
    pub status: String,
    /// When completed: `success`, `failure`, `cancelled`, `skipped`…
    pub conclusion: Option<String>,
}

impl Check {
    pub fn parse(v: &Value) -> Check {
        Check {
            name: s(v, "name"),
            status: s(v, "status"),
            conclusion: v
                .get("conclusion")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    }
}

/// `(owner, name)` of a GitHub remote address, for https and ssh forms alike.
pub fn parse_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, name) = rest.split_once('/')?;
    let valid = |x: &str| {
        !x.is_empty()
            && x.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (valid(owner) && valid(name)).then(|| (owner.to_owned(), name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn remotes_in_every_form_are_understood() {
        let want = Some(("getnodalia".to_owned(), "codigo".to_owned()));
        for url in [
            "https://github.com/getnodalia/codigo",
            "https://github.com/getnodalia/codigo.git",
            "https://github.com/getnodalia/codigo/",
            "git@github.com:getnodalia/codigo.git",
            "ssh://git@github.com/getnodalia/codigo",
        ] {
            assert_eq!(parse_remote(url), want, "{url}");
        }
        for bad in [
            "",
            "https://gitlab.com/a/b",
            "https://github.com/a",
            "https://github.com/a/b c",
            "git@github.com:a/b/c/d x",
        ] {
            assert_eq!(parse_remote(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_pull_request_is_read_with_its_state_and_labels() {
        let v = json!({
            "number": 15, "title": "feat: debugger", "state": "closed", "merged_at": "2026-11-26T10:00:00Z",
            "draft": false, "user": {"login": "claude"}, "html_url": "https://github.com/a/b/pull/15",
            "head": {"ref": "feat", "sha": "abc"}, "base": {"ref": "main"},
            "labels": [{"name": "release"}], "comments": 2
        });
        let p = Pull::parse(&v, "a/b");
        assert_eq!(
            (p.state.as_str(), p.number, p.author.as_str()),
            ("merged", 15, "claude")
        );
        assert_eq!(
            (
                p.head_ref.as_str(),
                p.base_ref.as_str(),
                p.head_sha.as_str()
            ),
            ("feat", "main", "abc")
        );
        assert_eq!(p.labels, ["release"]);
    }

    #[test]
    fn a_search_result_names_its_repository_by_url() {
        let v = json!({"number": 3, "title": "t", "state": "open", "repository_url": "https://api.github.com/repos/getnodalia/codigo",
            "user": {"login": "d"}, "pull_request": {"merged_at": null}});
        let p = Pull::parse_search(&v);
        assert_eq!(
            (p.repo.as_str(), p.state.as_str()),
            ("getnodalia/codigo", "open")
        );
        let i = Issue::parse(&v, "");
        assert_eq!(i.repo, "getnodalia/codigo");
        assert!(Issue::is_pull(&v));
    }

    #[test]
    fn missing_and_wrong_fields_never_panic() {
        for v in [
            json!({}),
            json!(null),
            json!([1, 2]),
            json!({"number": "x", "user": 3, "labels": 7, "head": "no"}),
        ] {
            let _ = (
                Pull::parse(&v, ""),
                Pull::parse_search(&v),
                PullDetail::parse(&v, ""),
                Issue::parse(&v, ""),
            );
            let _ = (
                PullFile::parse(&v),
                Comment::parse_comment(&v),
                Comment::parse_review(&v),
                Check::parse(&v),
            );
            let _ = (RepoInfo::parse(&v), User::parse(&v));
        }
        assert_eq!(Pull::parse(&json!({}), "a/b").author, "ghost");
    }
}
