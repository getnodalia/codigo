// SPDX-License-Identifier: MIT OR Apache-2.0
//! GitLab's JSON, read into the few fields Código shows. Every field is optional on the wire as
//! far as this code is concerned: a missing one is an empty string or zero, never a panic.

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

fn username(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|u| u.get("username"))
        .and_then(Value::as_str)
        .unwrap_or("ghost")
        .to_owned()
}

fn labels(v: &Value) -> Vec<String> {
    v.get("labels")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|l| l.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// `group/project!12` from a `references` object, or empty.
fn full_reference(v: &Value) -> String {
    v.get("references")
        .and_then(|r| r.get("full"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub id: u64,
    pub username: String,
    pub name: String,
    pub web_url: String,
}

impl User {
    pub fn parse(v: &Value) -> User {
        User {
            id: n(v, "id"),
            username: s(v, "username"),
            name: s(v, "name"),
            web_url: s(v, "web_url"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: u64,
    /// `group/subgroup/project`.
    pub path: String,
    pub description: String,
    pub visibility: String,
    pub stars: u64,
    pub http_url: String,
    pub web_url: String,
    pub default_branch: String,
    pub last_activity: String,
    pub archived: bool,
}

impl Project {
    pub fn parse(v: &Value) -> Project {
        Project {
            id: n(v, "id"),
            path: s(v, "path_with_namespace"),
            description: s(v, "description"),
            visibility: s(v, "visibility"),
            stars: n(v, "star_count"),
            http_url: s(v, "http_url_to_repo"),
            web_url: s(v, "web_url"),
            default_branch: s(v, "default_branch"),
            last_activity: s(v, "last_activity_at"),
            archived: b(v, "archived"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeRequest {
    pub project_id: u64,
    pub iid: u64,
    pub title: String,
    pub description: String,
    /// `opened`, `merged`, `closed` or `locked`.
    pub state: String,
    pub draft: bool,
    pub author: String,
    pub source_branch: String,
    pub target_branch: String,
    pub web_url: String,
    pub updated_at: String,
    /// `group/project!12`.
    pub reference: String,
    pub labels: Vec<String>,
    pub comments: u64,
    pub has_conflicts: bool,
    /// The status of the latest pipeline (`success`, `failed`, `running`…), when there is one.
    pub pipeline: Option<String>,
}

impl MergeRequest {
    pub fn parse(v: &Value) -> MergeRequest {
        MergeRequest {
            project_id: n(v, "project_id"),
            iid: n(v, "iid"),
            title: s(v, "title"),
            description: s(v, "description"),
            state: s(v, "state"),
            draft: b(v, "draft") || b(v, "work_in_progress"),
            author: username(v, "author"),
            source_branch: s(v, "source_branch"),
            target_branch: s(v, "target_branch"),
            web_url: s(v, "web_url"),
            updated_at: s(v, "updated_at"),
            reference: full_reference(v),
            labels: labels(v),
            comments: n(v, "user_notes_count"),
            has_conflicts: b(v, "has_conflicts"),
            pipeline: v
                .get("head_pipeline")
                .and_then(|p| p.get("status"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub project_id: u64,
    pub iid: u64,
    pub title: String,
    pub description: String,
    pub state: String,
    pub author: String,
    pub labels: Vec<String>,
    pub web_url: String,
    pub updated_at: String,
    pub reference: String,
    pub comments: u64,
}

impl Issue {
    pub fn parse(v: &Value) -> Issue {
        Issue {
            project_id: n(v, "project_id"),
            iid: n(v, "iid"),
            title: s(v, "title"),
            description: s(v, "description"),
            state: s(v, "state"),
            author: username(v, "author"),
            labels: labels(v),
            web_url: s(v, "web_url"),
            updated_at: s(v, "updated_at"),
            reference: full_reference(v),
            comments: n(v, "user_notes_count"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub created_at: String,
    /// GitLab's own notes: "changed the description", "assigned to…".
    pub system: bool,
}

impl Note {
    pub fn parse(v: &Value) -> Note {
        Note {
            id: n(v, "id"),
            author: username(v, "author"),
            body: s(v, "body"),
            created_at: s(v, "created_at"),
            system: b(v, "system"),
        }
    }
}

/// One file of a merge request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub old_path: String,
    pub new_path: String,
    /// The unified diff of the file, without the `---`/`+++` header.
    pub diff: String,
    pub new_file: bool,
    pub deleted_file: bool,
    pub renamed: bool,
}

impl Change {
    pub fn parse(v: &Value) -> Change {
        Change {
            old_path: s(v, "old_path"),
            new_path: s(v, "new_path"),
            diff: s(v, "diff"),
            new_file: b(v, "new_file"),
            deleted_file: b(v, "deleted_file"),
            renamed: b(v, "renamed_file"),
        }
    }
}

/// The project path (`group/sub/project`) a git remote names on `host`, for either the HTTPS or
/// the SSH spelling; `None` for a remote on another host.
pub fn parse_remote(url: &str, host: &str) -> Option<String> {
    let url = url.trim();
    let rest = if let Some(r) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("ssh://"))
    {
        // Drop `user@` and a port.
        let r = r.split_once('@').map_or(r, |(_, after)| after);
        let (h, path) = r.split_once('/')?;
        let h = h.split(':').next().unwrap_or(h);
        let wanted = host.split(':').next().unwrap_or(host);
        h.eq_ignore_ascii_case(wanted).then_some(path)?
    } else {
        // scp-like: git@host:group/project.git
        let r = url.split_once('@').map_or(url, |(_, after)| after);
        let (h, path) = r.split_once(':')?;
        let wanted = host.split(':').next().unwrap_or(host);
        h.eq_ignore_ascii_case(wanted).then_some(path)?
    };
    let path = rest.trim_matches('/').trim_end_matches(".git");
    (path.contains('/') && !path.is_empty()).then(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_merge_request_is_read_with_what_is_there() {
        let v = json!({
            "project_id": 7, "iid": 12, "title": "Add themes", "state": "opened",
            "draft": true, "author": {"username": "daniel"},
            "source_branch": "themes", "target_branch": "main",
            "web_url": "https://gitlab.com/a/b/-/merge_requests/12",
            "references": {"full": "a/b!12"}, "labels": ["ui", "p1"],
            "user_notes_count": 3, "head_pipeline": {"status": "success"}
        });
        let mr = MergeRequest::parse(&v);
        assert_eq!((mr.project_id, mr.iid), (7, 12));
        assert!(mr.draft);
        assert_eq!(mr.author, "daniel");
        assert_eq!(mr.reference, "a/b!12");
        assert_eq!(mr.labels, vec!["ui", "p1"]);
        assert_eq!(mr.comments, 3);
        assert_eq!(mr.pipeline.as_deref(), Some("success"));
    }

    #[test]
    fn missing_and_wrongly_typed_fields_never_panic() {
        for v in [
            json!({}),
            json!(null),
            json!([]),
            json!({"iid": "x", "author": 3, "labels": 5}),
        ] {
            let mr = MergeRequest::parse(&v);
            assert_eq!(mr.iid, 0);
            assert_eq!(mr.author, "ghost");
            assert!(mr.labels.is_empty());
            let _ = (Issue::parse(&v), Note::parse(&v), Project::parse(&v));
            let _ = (Change::parse(&v), User::parse(&v));
        }
    }

    #[test]
    fn notes_and_changes_are_read() {
        let note = Note::parse(&json!({
            "id": 9, "body": "Looks good", "created_at": "2026-11-27T10:00:00Z",
            "system": false, "author": {"username": "ana"}
        }));
        assert_eq!(
            (note.id, note.author.as_str(), note.system),
            (9, "ana", false)
        );
        let change = Change::parse(&json!({
            "old_path": "a.rs", "new_path": "b.rs", "diff": "@@ -1 +1 @@\n-a\n+b\n",
            "renamed_file": true
        }));
        assert!(change.renamed && !change.new_file && !change.deleted_file);
        assert!(change.diff.starts_with("@@"));
    }

    #[test]
    fn an_old_server_that_says_work_in_progress_still_marks_a_draft() {
        assert!(MergeRequest::parse(&json!({"work_in_progress": true})).draft);
    }

    #[test]
    fn remotes_name_their_project_in_every_spelling() {
        let host = "gitlab.com";
        for url in [
            "https://gitlab.com/group/sub/proj.git",
            "https://oauth2:tok@gitlab.com/group/sub/proj",
            "git@gitlab.com:group/sub/proj.git",
            "ssh://git@gitlab.com:22/group/sub/proj.git",
        ] {
            assert_eq!(
                parse_remote(url, host).as_deref(),
                Some("group/sub/proj"),
                "{url}"
            );
        }
        assert_eq!(parse_remote("https://github.com/a/b.git", host), None);
        assert_eq!(parse_remote("git@github.com:a/b.git", host), None);
        assert_eq!(parse_remote("https://gitlab.com/justone", host), None);
        assert_eq!(parse_remote("", host), None);
        assert_eq!(
            parse_remote(
                "https://git.example.org:8443/a/b.git",
                "git.example.org:8443"
            )
            .as_deref(),
            Some("a/b")
        );
    }
}
