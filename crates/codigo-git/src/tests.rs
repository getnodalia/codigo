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

#[test]
fn a_gitlab_token_never_appears_in_an_error() {
    let auth = Auth::GitLabToken {
        host: "gitlab.com".into(),
        token: "glpat-supersecret".into(),
    };
    assert_eq!(
        run::scrub("fatal: glpat-supersecret rejected", &auth),
        "fatal: *** rejected"
    );
    assert_eq!(format!("{auth:?}"), "Auth::GitLabToken(…)");
}

fn committed(repo: &Repo, name: &str, text: &str, message: &str) {
    write(repo, name, text);
    repo.stage_all().expect("add");
    repo.commit(message).expect("commit");
}

#[test]
fn stash_sets_changes_aside_and_brings_them_back() {
    let (_d, repo) = repo();
    committed(&repo, "a.txt", "one\n", "first");
    assert!(repo.stashes().expect("list").is_empty());
    write(&repo, "a.txt", "changed\n");
    write(&repo, "new.txt", "fresh\n");
    repo.stash_push("half done").expect("push");
    assert!(repo.status().expect("status").changes.is_empty());
    assert!(!repo.root().join("new.txt").exists());
    let list = repo.stashes().expect("list");
    assert_eq!(list.len(), 1);
    assert!(list[0].message.contains("half done"), "{:?}", list[0]);
    assert!(repo.stash_show(0).expect("show").contains("+changed"));

    repo.stash_apply(0).expect("apply");
    assert_eq!(repo.status().expect("status").changes.len(), 2);
    assert_eq!(repo.stashes().expect("list").len(), 1, "apply keeps it");
    repo.discard(&repo.status().expect("s").changes[0])
        .expect("discard");
    repo.stash_drop(0).expect("drop");
    assert!(repo.stashes().expect("list").is_empty());
    assert!(repo.stash_pop(0).is_err());
}

#[test]
fn blame_names_the_commit_of_every_line() {
    let (_d, repo) = repo();
    committed(&repo, "a.txt", "one\ntwo\n", "first");
    committed(&repo, "a.txt", "one\ntwo\nthree\n", "add three");
    write(&repo, "a.txt", "one\nTWO\nthree\n");
    let b = repo.blame("a.txt").expect("blame");
    assert_eq!(b.len(), 3);
    assert_eq!(b[0].summary, "first");
    assert_eq!(b[2].summary, "add three");
    assert_eq!(b[0].author, "T");
    assert!(b[1].uncommitted(), "{:?}", b[1]);
    assert!(!b[0].uncommitted());
    assert!(repo.blame("missing.txt").is_err());
    assert!(repo.blame("--help").is_err());
}

#[test]
fn one_hunk_at_a_time_can_be_staged_unstaged_and_discarded() {
    let (_d, repo) = repo();
    let base: String = (1..=40).map(|i| format!("line {i}\n")).collect();
    committed(&repo, "f.txt", &base, "base");
    let edited = base
        .replace("line 3\n", "line three\n")
        .replace("line 38\n", "line thirty-eight\n");
    write(&repo, "f.txt", &edited);

    let diff = repo.hunks("f.txt", false).expect("hunks");
    assert_eq!(diff.hunks.len(), 2, "{diff:?}");
    assert_eq!((diff.hunks[0].added(), diff.hunks[0].removed()), (1, 1));

    // Stage only the second hunk.
    repo.stage_hunk("f.txt", 1).expect("stage hunk");
    let staged = repo.hunks("f.txt", true).expect("staged");
    assert_eq!(staged.hunks.len(), 1);
    assert!(staged.hunks[0]
        .lines
        .iter()
        .any(|l| l == "+line thirty-eight"));
    let rest = repo.hunks("f.txt", false).expect("rest");
    assert_eq!(rest.hunks.len(), 1);
    assert!(rest.hunks[0].lines.iter().any(|l| l == "+line three"));

    // Take it back out.
    repo.unstage_hunk("f.txt", 0).expect("unstage hunk");
    assert!(repo.hunks("f.txt", true).expect("staged").hunks.is_empty());
    assert_eq!(repo.hunks("f.txt", false).expect("all").hunks.len(), 2);

    // Discard the first hunk from the folder.
    repo.discard_hunk("f.txt", 0).expect("discard hunk");
    let now = fs::read_to_string(repo.root().join("f.txt")).expect("read");
    assert!(now.contains("line 3\n") && now.contains("line thirty-eight\n"));

    // A stale index is an error, not a panic.
    assert!(repo.stage_hunk("f.txt", 9).is_err());
    assert!(repo.stage_hunk("--help", 0).is_err());
    assert!(repo
        .hunks("nope.txt", false)
        .expect("none")
        .hunks
        .is_empty());
}

#[test]
fn conflict_blocks_are_found_and_resolved_by_side() {
    let text = "top\n<<<<<<< HEAD\nours 1\nours 2\n=======\ntheirs\n>>>>>>> feature\nmiddle\n\
                <<<<<<< HEAD\na\n||||||| base\nb\n=======\nc\n>>>>>>> feature\nend\n";
    let all = conflicts(text);
    assert_eq!(all.len(), 2);
    assert_eq!((all[0].start, all[0].mid, all[0].end), (1, 4, 6));
    assert_eq!(all[0].ours_label, "HEAD");
    assert_eq!(all[0].theirs_label, "feature");
    assert_eq!(all[1].base, Some(10));
    assert_eq!(side_text(text, &all[0], Side::Theirs), "theirs\n");

    let ours = resolve(text, 0, Side::Ours).expect("ours");
    assert!(ours.starts_with("top\nours 1\nours 2\nmiddle\n<<<<<<<"));
    let theirs = resolve(&ours, 0, Side::Theirs).expect("theirs");
    assert_eq!(theirs, "top\nours 1\nours 2\nmiddle\nc\nend\n");
    assert!(conflicts(&theirs).is_empty());
    let both = resolve(text, 1, Side::Both).expect("both");
    assert!(both.contains("a\nc\nend\n"), "{both}");
    assert_eq!(resolve(text, 5, Side::Ours), None);

    // Seven characters exactly, and CRLF files keep their endings.
    assert!(conflicts("<<<<<<<< x\n=======\n>>>>>>>> y\n").is_empty());
    let crlf = "<<<<<<< HEAD\r\nx\r\n=======\r\ny\r\n>>>>>>> b\r\n";
    assert_eq!(resolve(crlf, 0, Side::Both).as_deref(), Some("x\r\ny\r\n"));
    // An unfinished block is not a conflict.
    assert!(conflicts("<<<<<<< HEAD\nx\n=======\ny\n").is_empty());
}

#[test]
fn a_real_merge_conflict_can_be_resolved_and_aborted() {
    let (_d, repo) = repo();
    committed(&repo, "a.txt", "base\n", "base");
    let main = repo.status().expect("s").branch.expect("branch");
    repo.create_branch("other").expect("branch");
    committed(&repo, "a.txt", "theirs\n", "other change");
    repo.checkout(&main).expect("back");
    committed(&repo, "a.txt", "ours\n", "main change");

    assert!(repo.merge("--bad").is_err());
    assert!(matches!(repo.merge("other"), Err(Error::Failed(_))));
    assert!(repo.merging());
    let s = repo.status().expect("status");
    let c = s.changes.iter().find(|c| c.path == "a.txt").expect("a");
    assert_eq!(c.kind(), Kind::Conflicted);

    repo.merge_abort().expect("abort");
    assert!(!repo.merging());
    assert_eq!(
        fs::read_to_string(repo.root().join("a.txt")).expect("r"),
        "ours\n"
    );

    // Again, this time resolving by hand.
    assert!(repo.merge("other").is_err());
    let text = fs::read_to_string(repo.root().join("a.txt")).expect("r");
    let fixed = resolve(&text, 0, Side::Theirs).expect("resolve");
    fs::write(repo.root().join("a.txt"), fixed).expect("w");
    repo.stage(&["a.txt".into()]).expect("stage");
    repo.commit("merged").expect("commit");
    assert!(!repo.merging());
    assert_eq!(
        fs::read_to_string(repo.root().join("a.txt")).expect("r"),
        "theirs\n"
    );
}
