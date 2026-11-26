// SPDX-License-Identifier: MIT OR Apache-2.0
//! The calls Código makes.

use serde_json::{json, Value};

use crate::http::{request, Method};
use crate::model::{Check, Comment, Issue, Pull, PullDetail, PullFile, RepoInfo, User};
use crate::{Error, Result};

/// Which pull requests to look for across GitHub.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Opened by me.
    Mine,
    /// Waiting for my review.
    ReviewRequested,
    /// Assigned to me.
    Assigned,
}

impl Scope {
    fn query(self) -> &'static str {
        match self {
            Scope::Mine => "is:pr is:open author:@me archived:false",
            Scope::ReviewRequested => "is:pr is:open review-requested:@me archived:false",
            Scope::Assigned => "is:pr is:open assignee:@me archived:false",
        }
    }
}

/// A review's verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewEvent {
    Approve,
    RequestChanges,
    Comment,
}

impl ReviewEvent {
    fn wire(self) -> &'static str {
        match self {
            ReviewEvent::Approve => "APPROVE",
            ReviewEvent::RequestChanges => "REQUEST_CHANGES",
            ReviewEvent::Comment => "COMMENT",
        }
    }
}

const API: &str = "https://api.github.com";

/// A signed-in client.
#[derive(Clone)]
pub struct Api {
    token: String,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Api(…)")
    }
}

/// Owner and repository names are path segments: only plain characters pass.
fn segment(name: &str) -> Result<&str> {
    let ok = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(name)
    } else {
        Err(Error::Unexpected(format!(
            "`{name}` is not a valid owner or repository name"
        )))
    }
}

fn array(v: Value) -> Result<Vec<Value>> {
    match v {
        Value::Array(a) => Ok(a),
        _ => Err(Error::Unexpected("expected a list".into())),
    }
}

impl Api {
    pub fn new(token: impl Into<String>) -> Api {
        Api {
            token: token.into(),
        }
    }

    fn get(&self, path: &str) -> Result<Value> {
        request(
            Method::Get,
            &format!("{API}{path}"),
            Some(&self.token),
            None,
        )
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value> {
        request(
            Method::Post,
            &format!("{API}{path}"),
            Some(&self.token),
            Some(body),
        )
    }

    /// Who the token belongs to; also the check that it works.
    pub fn user(&self) -> Result<User> {
        Ok(User::parse(&self.get("/user")?))
    }

    /// The repositories the person can reach, most recently pushed first.
    pub fn repos(&self) -> Result<Vec<RepoInfo>> {
        let v = self.get("/user/repos?per_page=100&sort=pushed&affiliation=owner,collaborator,organization_member")?;
        Ok(array(v)?.iter().map(RepoInfo::parse).collect())
    }

    /// Open pull requests of one repository.
    pub fn pulls(&self, owner: &str, repo: &str) -> Result<Vec<Pull>> {
        let (owner, repo) = (segment(owner)?, segment(repo)?);
        let v = self.get(&format!(
            "/repos/{owner}/{repo}/pulls?state=open&per_page=50&sort=updated&direction=desc"
        ))?;
        let full = format!("{owner}/{repo}");
        Ok(array(v)?.iter().map(|p| Pull::parse(p, &full)).collect())
    }

    /// Open issues of one repository (pull requests, which GitHub mixes in, are left out).
    pub fn issues(&self, owner: &str, repo: &str) -> Result<Vec<Issue>> {
        let (owner, repo) = (segment(owner)?, segment(repo)?);
        let v = self.get(&format!(
            "/repos/{owner}/{repo}/issues?state=open&per_page=50&sort=updated"
        ))?;
        let full = format!("{owner}/{repo}");
        Ok(array(v)?
            .iter()
            .filter(|i| !Issue::is_pull(i))
            .map(|i| Issue::parse(i, &full))
            .collect())
    }

    /// Pull requests across GitHub that concern the person.
    pub fn search_pulls(&self, scope: Scope) -> Result<Vec<Pull>> {
        let q = scope
            .query()
            .replace(' ', "+")
            .replace('@', "%40")
            .replace(':', "%3A");
        let v = self.get(&format!(
            "/search/issues?q={q}&sort=updated&order=desc&per_page=50"
        ))?;
        let items = v.get("items").cloned().unwrap_or(Value::Null);
        Ok(array(items)?.iter().map(Pull::parse_search).collect())
    }

    pub fn pull(&self, owner: &str, repo: &str, number: u64) -> Result<PullDetail> {
        let (owner, repo) = (segment(owner)?, segment(repo)?);
        let v = self.get(&format!("/repos/{owner}/{repo}/pulls/{number}"))?;
        Ok(PullDetail::parse(&v, &format!("{owner}/{repo}")))
    }

    pub fn pull_files(&self, owner: &str, repo: &str, number: u64) -> Result<Vec<PullFile>> {
        let (owner, repo) = (segment(owner)?, segment(repo)?);
        let v = self.get(&format!(
            "/repos/{owner}/{repo}/pulls/{number}/files?per_page=100"
        ))?;
        Ok(array(v)?.iter().map(PullFile::parse).collect())
    }

    /// The conversation of a pull request or issue: comments and, for pull requests, reviews,
    /// oldest first.
    pub fn conversation(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        pull: bool,
    ) -> Result<Vec<Comment>> {
        let (owner, repo) = (segment(owner)?, segment(repo)?);
        let mut all: Vec<Comment> = array(self.get(&format!(
            "/repos/{owner}/{repo}/issues/{number}/comments?per_page=100"
        ))?)?
        .iter()
        .map(Comment::parse_comment)
        .collect();
        if pull {
            let reviews = array(self.get(&format!(
                "/repos/{owner}/{repo}/pulls/{number}/reviews?per_page=100"
            ))?)?;
            all.extend(reviews.iter().map(Comment::parse_review).filter(|c| {
                !c.body.is_empty()
                    || !matches!(&c.kind, crate::CommentKind::Review(s) if s == "COMMENTED")
            }));
        }
        all.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(all)
    }

    /// CI checks on a commit.
    pub fn checks(&self, owner: &str, repo: &str, sha: &str) -> Result<Vec<Check>> {
        let (owner, repo) = (segment(owner)?, segment(repo)?);
        if sha.is_empty() || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::Unexpected("that is not a commit".into()));
        }
        let v = self.get(&format!(
            "/repos/{owner}/{repo}/commits/{sha}/check-runs?per_page=100"
        ))?;
        let runs = v.get("check_runs").cloned().unwrap_or(Value::Null);
        Ok(array(runs)?.iter().map(Check::parse).collect())
    }

    /// Say something on a pull request or issue.
    pub fn comment(&self, owner: &str, repo: &str, number: u64, body: &str) -> Result<()> {
        let (owner, repo) = (segment(owner)?, segment(repo)?);
        if body.trim().is_empty() {
            return Err(Error::Api("write something first".into()));
        }
        self.post(
            &format!("/repos/{owner}/{repo}/issues/{number}/comments"),
            &json!({ "body": body }),
        )
        .map(drop)
    }

    /// Submit a review of a pull request.
    pub fn review(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        event: ReviewEvent,
        body: &str,
    ) -> Result<()> {
        let (owner, repo) = (segment(owner)?, segment(repo)?);
        if event != ReviewEvent::Approve && body.trim().is_empty() {
            return Err(Error::Api(
                "a review that is not an approval needs a comment".into(),
            ));
        }
        self.post(
            &format!("/repos/{owner}/{repo}/pulls/{number}/reviews"),
            &json!({ "event": event.wire(), "body": body }),
        )
        .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_could_change_the_path_are_refused() {
        for bad in ["", ".", "..", "a/b", "a b", "a?x=1", "a#", "../x", "%2e"] {
            assert!(segment(bad).is_err(), "{bad}");
        }
        for good in ["getnodalia", "codigo", "my-repo_1.x"] {
            assert!(segment(good).is_ok());
        }
        let api = Api::new("ghp_secretsecret");
        assert!(api.pulls("a/b", "c").is_err());
        assert!(api.checks("a", "b", "../x").is_err());
        assert_eq!(format!("{api:?}"), "Api(…)");
    }

    #[test]
    fn empty_comments_and_unexplained_rejections_are_refused_before_any_request() {
        let api = Api::new("ghp_secretsecret");
        assert!(api.comment("a", "b", 1, "  ").is_err());
        assert!(api
            .review("a", "b", 1, ReviewEvent::RequestChanges, "")
            .is_err());
        assert!(api.review("a", "b", 1, ReviewEvent::Comment, " ").is_err());
    }

    #[test]
    fn search_queries_are_encoded() {
        let q = Scope::ReviewRequested
            .query()
            .replace(' ', "+")
            .replace('@', "%40")
            .replace(':', "%3A");
        assert!(!q.contains(' ') && !q.contains('@') && !q.contains(':'));
    }
}
