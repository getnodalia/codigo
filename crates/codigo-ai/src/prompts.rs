// SPDX-License-Identifier: MIT OR Apache-2.0
//! What Código asks, and how it reads the answer.

/// The text sent along as context is capped so one huge file cannot become one huge bill.
pub const MAX_CONTEXT: usize = 48_000;

/// Always the first instruction.
pub const SYSTEM: &str = "You are the assistant inside Código, a code editor. You can see the \
project the person has open: the file in the editor, their selection, the other open files, an \
outline of the folder and any file they mention with @. Answer briefly and concretely, about that \
code. Never invent APIs or files; say when you are unsure.\n\n\
To change code in a project file, write one fenced block per file whose first line is `edit` and \
the file's path relative to the project folder, holding one or more pairs of the old lines and the \
new ones, copied exactly from the file:\n\
```edit src/main.rs\n<<<<<<< SEARCH\nold lines\n=======\nnew lines\n>>>>>>> REPLACE\n```\n\
Each SEARCH must appear exactly once in the file, so include enough lines around the change. To \
create a new file, write a block whose first line is `create` and the path, then the whole \
file. Say in a sentence or two what changed and why. Do not paste a whole file back to change a \
few lines. When the message gives you code to work on and asks for a replacement, reply with the \
complete replacement in one plain fenced code block instead.";

/// A job the person can ask for from the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    Explain,
    Fix,
    Refactor,
    Document,
    Test,
}

impl Task {
    fn instruction(self) -> &'static str {
        match self {
            Task::Explain => "Explain what this code does and point out anything surprising.",
            Task::Fix => {
                "Find the bug or the weakest part of this code and fix it. Explain the fix."
            }
            Task::Refactor => {
                "Refactor this code to be clearer and simpler without changing what it does."
            }
            Task::Document => {
                "Add documentation comments in this language's usual style. Do not change the code."
            }
            Task::Test => "Write tests for this code in the project's usual test style.",
        }
    }
}

/// Cut `text` to at most `max` bytes at a character boundary.
pub fn clip(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or("")
}

/// A fenced block holding code to talk about.
pub fn fenced(language: &str, code: &str) -> String {
    let code = clip(code, MAX_CONTEXT);
    // A fence longer than any run of backticks inside cannot be closed early by the code.
    let mut longest = 0usize;
    let mut run = 0usize;
    for c in code.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}{language}\n{code}\n{fence}")
}

/// The question for a task about `code` in `file`.
pub fn ask(task: Task, file: &str, language: &str, code: &str) -> String {
    format!(
        "{}\n\nFile: {file}\n\n{}",
        task.instruction(),
        fenced(language, code)
    )
}

/// The first fenced code block of an answer: the replacement text, without the fences.
pub fn first_code_block(reply: &str) -> Option<String> {
    let mut lines = reply.lines();
    let mut fence_len = 0usize;
    for line in lines.by_ref() {
        let t = line.trim_start();
        let n = t.chars().take_while(|c| *c == '`').count();
        if n >= 3 {
            fence_len = n;
            break;
        }
    }
    if fence_len == 0 {
        return None;
    }
    let mut code = Vec::new();
    for line in lines {
        let t = line.trim();
        let n = t.chars().take_while(|c| *c == '`').count();
        if n >= fence_len && t.chars().all(|c| c == '`') {
            return Some(code.join("\n"));
        }
        code.push(line);
    }
    // An answer cut off before the closing fence still has its code.
    (!code.is_empty()).then(|| code.join("\n"))
}
