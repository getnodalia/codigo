// SPDX-License-Identifier: MIT OR Apache-2.0
//! The calls Código makes.

use serde_json::{json, Value};

use crate::http::{request, Answer, Method};
use crate::model::{Change, Issue, MergeRequest, Note, Project, User};
use crate::{Error, Result};

/// The address of GitLab's own service.
pub const DEFAULT_HOST: &str = "gitlab.com";

/// How many pages of a list are read at most, so a huge account cannot keep a request going.
const MAX_PAGES: u32 = 4;
const PER_PAGE: u32 = 50;

/// Which merge requests or issues to look for across GitLab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Opened by me.
    Mine,
    /// Assigned to me.
    Assigned,
    /// Waiting for my review (merge requests only).
    ReviewRequested,
}

/// What a new merge request says.
#[derive(Clone, Copy, Debug)]
pub struct NewMergeRequest<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub source_branch: &'a str,
    pub target_branch: &'a str,
    pub draft: bool,
}

/// Whether `host` is a plain server name, with an optional port: letters, digits, dots, hyphens.
/// No scheme, path, user or query can get in, so a request is only ever made to that server.
pub fn valid_host(host: &str) -> bool {
    let (name, port) = match host.split_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (host, None),
    };
    let name_ok = !name.is_empty()
        && name.len() <= 253
        && name.contains('.')
        && !name.starts_with(['.', '-'])
        && !name.ends_with(['.', '-'])
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    let port_ok = port.is_none_or(|p| {
        !p.is_empty() && p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()) && p != "0"
    });
    name_ok && port_ok
}

fn array(v: Value) -> Result<Vec<Value>> {
    match v {
        Value::Array(a) => Ok(a),
        _ => Err(Error::Unexpected("expected a list".into())),
    }
}

/// A path such as `group/sub/project` as the single segment GitLab wants in a URL.
fn encode_path(path: &str) -> Result<String> {
    let ok = !path.is_empty()
        && path.len() <= 255
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'));
    if ok {
        Ok(path.replace('/', "%2F"))
    } else {
        Err(Error::Unexpected(format!(
            "`{path}` is not a valid project path"
        )))
    }
}

/// A value for a query string.
fn query_value(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A signed-in client for one GitLab server.
#[derive(Clone)]
pub struct Api {
    host: String,
    token: String,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api").field("host", &self.host).finish()
    }
}

impl Api {
    /// A client for `host`, which must pass [`valid_host`].
    pub fn new(host: &str, token: impl Into<String>) -> Result<Api> {
        let host = host.trim().to_lowercase();
        if !valid_host(&host) {
            return Err(Error::Unexpected(format!(
                "`{host}` is not a server address such as gitlab.com"
            )));
        }
        Ok(Api {
            host,
            token: token.into(),
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    fn get(&self, path: &str) -> Result<Answer> {
        request(Method::Get, &self.host, path, Some(&self.token), None)
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value> {
        request(
            Method::Post,
            &self.host,
            path,
            Some(&self.token),
            Some(body),
        )
        .map(|a| a.value)
    }

    /// Every item of a list, a few pages at most.
    fn list(&self, path: &str) -> Result<Vec<Value>> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let mut out = Vec::new();
        let mut page = 1;
        for _ in 0..MAX_PAGES {
            let answer = self.get(&format!("{path}{sep}per_page={PER_PAGE}&page={page}"))?;
            out.extend(array(answer.value)?);
            match answer.next_page {
                Some(next) if next > page => page = next,
                _ => break,
            }
        }
        Ok(out)
    }

    /// The person the token belongs to.
    pub fn current_user(&self) -> Result<User> {
        Ok(User::parse(&self.get("user")?.value))
    }

    /// The projects the person is a member of, the most recently active first.
    pub fn projects(&self) -> Result<Vec<Project>> {
        Ok(self
            .list("projects?membership=true&order_by=last_activity_at&archived=false")?
            .iter()
            .map(Project::parse)
            .collect())
    }

    /// One project, by its `group/project` path.
    pub fn project(&self, path: &str) -> Result<Project> {
        let path = encode_path(path)?;
        Ok(Project::parse(
            &self.get(&format!("projects/{path}"))?.value,
        ))
    }

    /// Open merge requests across GitLab.
    pub fn merge_requests(&self, scope: Scope, username: &str) -> Result<Vec<MergeRequest>> {
        let filter = match scope {
            Scope::Mine => "scope=created_by_me".to_owned(),
            Scope::Assigned => "scope=assigned_to_me".to_owned(),
            Scope::ReviewRequested => {
                format!("scope=all&reviewer_username={}", query_value(username))
            }
        };
        Ok(self
            .list(&format!("merge_requests?state=opened&{filter}"))?
            .iter()
            .map(MergeRequest::parse)
            .collect())
    }

    /// Open issues across GitLab.
    pub fn issues(&self, scope: Scope) -> Result<Vec<Issue>> {
        let filter = match scope {
            Scope::Mine => "scope=created_by_me",
            Scope::Assigned => "scope=assigned_to_me",
            Scope::ReviewRequested => "scope=all",
        };
        Ok(self
            .list(&format!("issues?state=opened&{filter}"))?
            .iter()
            .map(Issue::parse)
            .collect())
    }

    /// Open merge requests of one project.
    pub fn project_merge_requests(&self, project_id: u64) -> Result<Vec<MergeRequest>> {
        Ok(self
            .list(&format!(
                "projects/{project_id}/merge_requests?state=opened"
            ))?
            .iter()
            .map(MergeRequest::parse)
            .collect())
    }

    /// Open issues of one project.
    pub fn project_issues(&self, project_id: u64) -> Result<Vec<Issue>> {
        Ok(self
            .list(&format!("projects/{project_id}/issues?state=opened"))?
            .iter()
            .map(Issue::parse)
            .collect())
    }

    /// One merge request with its description and pipeline.
    pub fn merge_request(&self, project_id: u64, iid: u64) -> Result<MergeRequest> {
        Ok(MergeRequest::parse(
            &self
                .get(&format!("projects/{project_id}/merge_requests/{iid}"))?
                .value,
        ))
    }

    /// The files a merge request changes.
    pub fn changes(&self, project_id: u64, iid: u64) -> Result<Vec<Change>> {
        Ok(self
            .list(&format!("projects/{project_id}/merge_requests/{iid}/diffs"))?
            .iter()
            .map(Change::parse)
            .collect())
    }

    /// The conversation of a merge request, oldest first.
    pub fn merge_request_notes(&self, project_id: u64, iid: u64) -> Result<Vec<Note>> {
        self.notes(&format!("projects/{project_id}/merge_requests/{iid}/notes"))
    }

    /// The conversation of an issue, oldest first.
    pub fn issue_notes(&self, project_id: u64, iid: u64) -> Result<Vec<Note>> {
        self.notes(&format!("projects/{project_id}/issues/{iid}/notes"))
    }

    fn notes(&self, path: &str) -> Result<Vec<Note>> {
        Ok(self
            .list(&format!("{path}?sort=asc&order_by=created_at"))?
            .iter()
            .map(Note::parse)
            .collect())
    }

    /// Comment on a merge request.
    pub fn comment_on_merge_request(&self, project_id: u64, iid: u64, body: &str) -> Result<Note> {
        Ok(Note::parse(&self.post(
            &format!("projects/{project_id}/merge_requests/{iid}/notes"),
            &json!({ "body": body }),
        )?))
    }

    /// Comment on an issue.
    pub fn comment_on_issue(&self, project_id: u64, iid: u64, body: &str) -> Result<Note> {
        Ok(Note::parse(&self.post(
            &format!("projects/{project_id}/issues/{iid}/notes"),
            &json!({ "body": body }),
        )?))
    }

    /// Approve a merge request.
    pub fn approve(&self, project_id: u64, iid: u64) -> Result<()> {
        self.post(
            &format!("projects/{project_id}/merge_requests/{iid}/approve"),
            &json!({}),
        )
        .map(|_| ())
    }

    /// Open a merge request; the address of the new one.
    pub fn create_merge_request(&self, project_id: u64, new: NewMergeRequest) -> Result<String> {
        let title = if new.draft && !new.title.starts_with("Draft:") {
            format!("Draft: {}", new.title)
        } else {
            new.title.to_owned()
        };
        let made = self.post(
            &format!("projects/{project_id}/merge_requests"),
            &json!({
                "title": title,
                "description": new.description,
                "source_branch": new.source_branch,
                "target_branch": new.target_branch,
            }),
        )?;
        Ok(MergeRequest::parse(&made).web_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_server_names_pass() {
        for ok in [
            "gitlab.com",
            "git.example.org",
            "gitlab.my-company.co.uk",
            "git.example.org:8443",
        ] {
            assert!(valid_host(ok), "{ok}");
        }
        for bad in [
            "",
            "gitlab",
            "https://gitlab.com",
            "gitlab.com/",
            "gitlab.com/evil",
            "user@gitlab.com",
            "gitlab.com@evil.example",
            "gitlab.com:",
            "gitlab.com:0",
            "gitlab.com:99999x",
            ".gitlab.com",
            "gitlab..com",
            "gitlab.com?x=1",
            "gitlab.com#",
            "git lab.com",
            "gitlab.com\\evil",
        ] {
            assert!(!valid_host(bad), "{bad:?}");
        }
        assert!(Api::new("https://gitlab.com", "tok").is_err());
        assert!(Api::new(" GitLab.com ", "tok").is_ok_and(|a| a.host() == "gitlab.com"));
    }

    #[test]
    fn a_project_path_becomes_one_url_segment() {
        assert_eq!(
            encode_path("group/sub/proj").as_deref(),
            Ok("group%2Fsub%2Fproj")
        );
        for bad in ["", "a//b", "../x", "a/..", "a b/c", "a/b?x", "/a/b"] {
            assert!(encode_path(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn query_values_are_escaped() {
        assert_eq!(query_value("daniel-t_1.x"), "daniel-t_1.x");
        assert_eq!(query_value("a&b=c d"), "a%26b%3Dc%20d");
    }

    /// Reads a public project, its open merge requests and their diffs from gitlab.com with no
    /// token (notes need one, so they are not read): `cargo test -p codigo-gitlab -- --ignored`.
    #[test]
    #[ignore = "needs the network"]
    fn the_real_gitlab_answers_in_the_shape_the_models_read() {
        let get =
            |path: &str| request(Method::Get, DEFAULT_HOST, path, None, None).map(|a| a.value);
        let project = Project::parse(&get("projects/gitlab-org%2Fgitlab-runner").expect("project"));
        assert_eq!(project.path, "gitlab-org/gitlab-runner");
        assert!(project.http_url.ends_with(".git"));
        let mrs = get(&format!(
            "projects/{}/merge_requests?state=opened&per_page=3",
            project.id
        ))
        .expect("merge requests");
        let mr = MergeRequest::parse(mrs.get(0).expect("an open merge request"));
        assert!(mr.iid > 0 && !mr.title.is_empty() && !mr.web_url.is_empty());
        let diffs = get(&format!(
            "projects/{}/merge_requests/{}/diffs?per_page=3",
            mr.project_id, mr.iid
        ))
        .expect("diffs");
        assert!(diffs
            .as_array()
            .is_some_and(|a| a.iter().all(|c| !Change::parse(c).new_path.is_empty())));
    }

    #[test]
    fn the_debug_text_never_shows_the_token() {
        let api = Api::new("gitlab.com", "glpat-secret-token").expect("host is valid");
        assert!(!format!("{api:?}").contains("secret"));
    }
}
