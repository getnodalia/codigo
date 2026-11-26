use super::*;
use std::fs;

fn repo() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().expect("dir");
    let repo = Repo::init(dir.path()).expect("init");
    for (k, v) in [
        ("user.name", "T"),
        ("user.email", "t@example.com"),
        ("commit.gpgsign", "false"),
    ] {
        run::git(repo.root(), &["config", k, v], &Auth::None).expect("config");
    }
    (dir, repo)
}

fn write(repo: &Repo, name: &str, text: &str) {
    fs::write(repo.root().join(name), text).expect("write");
}

#[test]
fn the_whole_cycle_stage_commit_branch() {
    let (_d, repo) = repo();
    write(&repo, "a.txt", "one\n");
    write(&repo, "b.txt", "two\n");
    let s = repo.status().expect("status");
    assert_eq!(s.changes.len(), 2);
    assert!(s
        .changes
        .iter()
        .all(|c| c.kind() == Kind::Untracked && !c.staged()));

    repo.stage(&["a.txt".into()]).expect("stage");
    let s = repo.status().expect("status");
    assert_eq!(
        s.staged().map(|c| c.path.as_str()).collect::<Vec<_>>(),
        ["a.txt"]
    );
    assert_eq!(s.staged().next().map(Change::kind), Some(Kind::Added));
    assert!(repo.commit("   ").is_err());
    let short = repo.commit("first").expect("commit");
    assert!(!short.is_empty());

    write(&repo, "a.txt", "one\nmore\n");
    let s = repo.status().expect("status");
    let a = s.changes.iter().find(|c| c.path == "a.txt").expect("a");
    assert_eq!(
        (a.kind(), a.staged(), a.unstaged()),
        (Kind::Modified, false, true)
    );
    assert!(repo.diff(a, false).expect("diff").contains("+more"));
    let b = s.changes.iter().find(|c| c.path == "b.txt").expect("b");
    assert!(repo
        .diff(b, false)
        .expect("untracked diff")
        .contains("+two"));

    repo.stage(&["a.txt".into()]).expect("stage");
    assert!(repo.diff(a, true).expect("staged diff").contains("+more"));
    repo.unstage(&["a.txt".into()]).expect("unstage");
    assert!(repo.status().expect("s").staged().next().is_none());

    let log = repo.log(10).expect("log");
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "first");

    repo.create_branch("feature/x").expect("branch");
    let names: Vec<_> = repo
        .branches()
        .expect("branches")
        .into_iter()
        .map(|b| (b.name, b.current))
        .collect();
    assert!(names.contains(&("feature/x".to_owned(), true)));
    let s = repo.status().expect("status");
    assert_eq!(s.branch.as_deref(), Some("feature/x"));
    assert!(repo.checkout("--force").is_err());
    assert!(repo.create_branch("has space").is_err());
}

#[test]
fn discard_restores_a_file_and_deletes_an_untracked_one() {
    let (_d, repo) = repo();
    write(&repo, "a.txt", "one\n");
    repo.stage_all().expect("add");
    repo.commit("c").expect("commit");
    write(&repo, "a.txt", "changed\n");
    write(&repo, "new.txt", "x\n");
    let s = repo.status().expect("status");
    for c in &s.changes {
        repo.discard(c).expect("discard");
    }
    assert!(repo.status().expect("status").changes.is_empty());
    assert_eq!(
        fs::read_to_string(repo.root().join("a.txt")).expect("read"),
        "one\n"
    );
    assert!(!repo.root().join("new.txt").exists());
}

#[test]
fn a_rename_and_a_conflict_are_read_from_the_porcelain() {
    let text =
        "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
        2 R. N... 100644 100644 100644 aaa bbb R100 new name.txt\0old name.txt\0\
        u UU N... 100644 100644 100644 100644 a b c both.txt\0\
        1 .M N... 100644 100644 100644 a b src/lib.rs\0? untracked file.txt\0";
    let s = status::parse(text);
    assert_eq!(
        (
            s.branch.as_deref(),
            s.upstream.as_deref(),
            s.ahead,
            s.behind
        ),
        (Some("main"), Some("origin/main"), 2, 1)
    );
    let find = |p: &str| s.changes.iter().find(|c| c.path == p).expect(p);
    assert_eq!(find("new name.txt").kind(), Kind::Renamed);
    assert_eq!(find("new name.txt").from.as_deref(), Some("old name.txt"));
    assert_eq!(find("both.txt").kind(), Kind::Conflicted);
    assert!(find("both.txt").unstaged() && !find("both.txt").staged());
    assert_eq!(find("src/lib.rs").kind(), Kind::Modified);
    assert_eq!(find("untracked file.txt").kind(), Kind::Untracked);
    assert!(status::parse("").changes.is_empty());
    assert_eq!(status::parse("# branch.head (detached)\0").branch, None);
}

#[test]
fn clone_and_push_pull_work_between_two_local_repositories() {
    let (d, origin) = repo();
    write(&origin, "a.txt", "one\n");
    origin.stage_all().expect("add");
    origin.commit("first").expect("commit");
    // Local paths are not clonable through `clone`, which only takes remote addresses…
    assert!(clone(&origin.root().to_string_lossy(), d.path(), "x", &Auth::None).is_err());
    // …so clone the way git itself would, then use the repo API on the copy.
    let copy = d.path().join("copy");
    run::git(
        d.path(),
        &["clone", "--quiet", &origin.root().to_string_lossy(), "copy"],
        &Auth::None,
    )
    .expect("clone");
    let copy = Repo::discover(&copy).expect("copy is a repo");
    for (k, v) in [
        ("user.name", "T"),
        ("user.email", "t@example.com"),
        ("commit.gpgsign", "false"),
    ] {
        run::git(copy.root(), &["config", k, v], &Auth::None).expect("config");
    }
    write(&origin, "b.txt", "two\n");
    origin.stage_all().expect("add");
    origin.commit("second").expect("commit");
    copy.fetch(&Auth::None).expect("fetch");
    assert_eq!(copy.status().expect("s").behind, 1);
    copy.pull(&Auth::None).expect("pull");
    assert!(copy.root().join("b.txt").exists());
    // Pushing to a checked-out branch is refused by git; the error comes back as text.
    write(&copy, "c.txt", "three\n");
    copy.stage_all().expect("add");
    copy.commit("third").expect("commit");
    assert_eq!(copy.status().expect("s").ahead, 1);
    assert!(matches!(copy.push(&Auth::None), Err(Error::Failed(_))));
}

#[test]
fn clone_addresses_are_checked_and_names_derived() {
    let d = tempfile::tempdir().expect("dir");
    for bad in [
        "--upload-pack=x",
        "ext::sh -c id",
        "/tmp/repo",
        "file:///tmp/r",
        "https://a b",
    ] {
        assert!(clone(bad, d.path(), "x", &Auth::None).is_err(), "{bad}");
    }
    assert!(clone("https://github.com/a/b.git", d.path(), "../x", &Auth::None).is_err());
    assert_eq!(folder_name("https://github.com/a/b.git"), "b");
    assert_eq!(folder_name("git@github.com:a/b"), "b");
    assert_eq!(folder_name("https://github.com/a/b/"), "b");
    assert_eq!(
        strip_credentials("https://user:tok@github.com/a/b"),
        "https://github.com/a/b"
    );
    assert_eq!(
        strip_credentials("https://github.com/a/b"),
        "https://github.com/a/b"
    );
    assert_eq!(
        strip_credentials("git@github.com:a/b"),
        "git@github.com:a/b"
    );
}

#[test]
fn a_token_never_appears_in_an_error() {
    let d = tempfile::tempdir().expect("dir");
    let auth = Auth::GitHubToken("ghp_supersecrettoken".into());
    assert_eq!(
        run::scrub("fatal: ghp_supersecrettoken rejected", &auth),
        "fatal: *** rejected"
    );
    assert_eq!(format!("{auth:?}"), "Auth::GitHubToken(…)");
    let err = run::git(d.path(), &["rev-parse", "--show-toplevel"], &auth).expect_err("not a repo");
    assert!(!err.to_string().contains("ghp_"));
}
