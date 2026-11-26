// SPDX-License-Identifier: MIT OR Apache-2.0
//! Debug a real program with the adapters installed on the machine. A test skips itself when its
//! adapter or the C compiler is missing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use codigo_dap::{adapters_for, AdapterKind, Command as Cmd, Debugger, LaunchSpec, Update};

const SOURCE: &str = "#include <stdio.h>\n\
int twice(int n) {\n\
    int r = n * 2;\n\
    return r;\n\
}\n\
int main(void) {\n\
    int x = 21;\n\
    int y = twice(x);\n\
    printf(\"answer %d\\n\", y);\n\
    return 0;\n\
}\n";

fn compile(dir: &Path) -> Option<std::path::PathBuf> {
    let src = dir.join("prog.c");
    std::fs::write(&src, SOURCE).ok()?;
    let exe = dir.join("prog");
    let ok = Command::new("cc")
        .args(["-g", "-O0", "-o"])
        .arg(&exe)
        .arg(&src)
        .status()
        .ok()?
        .success();
    ok.then_some(exe)
}

fn next(d: &Debugger, what: &str) -> Update {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(u) = d.poll() {
            return u;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Run to the breakpoint, look at the frame and the variables, step, continue, and finish.
fn drive(want: AdapterKind) {
    let Some(adapter) = adapters_for("c")
        .ok()
        .and_then(|all| all.into_iter().find(|a| a.kind == want))
    else {
        eprintln!("{want:?} is not installed: skipped");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(exe) = compile(dir.path()) else {
        eprintln!("no C compiler: skipped");
        return;
    };
    let src = dir.path().join("prog.c");
    let mut breakpoints = BTreeMap::new();
    breakpoints.insert(src.clone(), vec![8]);
    let d = Debugger::start(
        LaunchSpec {
            adapter,
            program: exe,
            args: Vec::new(),
            cwd: dir.path().to_path_buf(),
            breakpoints,
        },
        None,
    );

    let mut output = String::new();
    let stop = loop {
        match next(&d, "the first stop") {
            Update::Stopped(s) => break s,
            Update::Output { text, .. } => output.push_str(&text),
            Update::Ended { error, .. } => panic!("ended early: {error:?}"),
            _ => {}
        }
    };
    let top = &stop.frames[0];
    assert_eq!(top.line, 8, "stopped on the breakpoint's line: {top:?}");
    assert!(top.name.contains("main"), "{top:?}");
    let vars: Vec<_> = stop.scopes.iter().flat_map(|s| s.vars.iter()).collect();
    let x = vars.iter().find(|v| v.name == "x").expect("x is in scope");
    assert_eq!(x.value.trim(), "21");

    // Step into twice(), then out again.
    d.send(Cmd::StepInto);
    let inside = loop {
        match next(&d, "the step into") {
            Update::Stopped(s) => break s,
            Update::Ended { error, .. } => panic!("ended early: {error:?}"),
            _ => {}
        }
    };
    assert!(
        inside.frames[0].name.contains("twice"),
        "{:?}",
        inside.frames
    );
    assert!(inside.frames.len() >= 2);

    d.send(Cmd::Continue);
    let (code, error);
    loop {
        match next(&d, "the end") {
            Update::Output { text, .. } => output.push_str(&text),
            Update::Ended {
                exit_code,
                error: e,
            } => {
                code = exit_code;
                error = e;
                break;
            }
            _ => {}
        }
    }
    assert_eq!(error, None);
    assert!(code == Some(0) || code.is_none(), "exit code {code:?}");
    assert!(output.contains("answer 42"), "program output: {output:?}");
}

#[test]
fn lldb_dap_debugs_a_c_program() {
    drive(AdapterKind::Lldb);
}

#[test]
fn gdb_debugs_a_c_program() {
    drive(AdapterKind::Gdb);
}
