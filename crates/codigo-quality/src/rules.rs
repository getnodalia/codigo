// SPDX-License-Identifier: MIT OR Apache-2.0
// codigo-ignore-file
//! The rules, and what a finding is.

/// How much a finding matters. Ordered: later is worse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Info,
    Minor,
    Major,
    Critical,
    Blocker,
}

/// What kind of problem a rule finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Code that can fail or panic.
    Bug,
    /// Code an attacker can abuse.
    Vulnerability,
    /// Code that is safe only in the right hands: worth a look.
    Hotspot,
    /// A key, a token or a password written into the source.
    Secret,
    /// Code that is hard to read, change or trust.
    Smell,
    /// A rule of Nodalia's engineering standards, for apps that follow them.
    Convention,
}

/// One rule. `title` is an English sentence the interface translates; when it has a `{}` the
/// finding's `detail` fills it (a number, a name, a place).
#[derive(Debug, PartialEq, Eq)]
pub struct Rule {
    pub id: &'static str,
    pub kind: Kind,
    pub severity: Severity,
    pub title: &'static str,
    pub advice: &'static str,
}

/// One place a rule fired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static Rule,
    pub severity: Severity,
    /// 0-based.
    pub line: usize,
    /// Byte columns inside the line, `start..end`.
    pub start: usize,
    pub end: usize,
    /// The value for the title's `{}`.
    pub detail: Option<String>,
}

impl Finding {
    pub(crate) fn new(rule_id: &str, line: usize, start: usize, end: usize) -> Option<Finding> {
        let rule = rule(rule_id)?;
        Some(Finding {
            rule,
            severity: rule.severity,
            line,
            start,
            end,
            detail: None,
        })
    }

    pub(crate) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub(crate) fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }
}

pub fn rule(id: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.id == id)
}

use Kind::{Bug, Convention, Hotspot, Secret, Smell, Vulnerability};
use Severity::{Blocker, Critical, Info, Major, Minor};

macro_rules! rule {
    ($id:literal, $kind:expr, $sev:expr, $title:literal, $advice:literal) => {
        Rule {
            id: $id,
            kind: $kind,
            severity: $sev,
            title: $title,
            advice: $advice,
        }
    };
}

pub const RULES: &[Rule] = &[
    rule!(
        "private-key",
        Secret,
        Blocker,
        "A private key is in the source",
        "Remove the key, rotate it, and load it from a file or a secret store."
    ),
    rule!(
        "cloud-token",
        Secret,
        Blocker,
        "A cloud or service token is in the source",
        "Revoke the token, create a new one and keep it out of the repository."
    ),
    rule!(
        "hardcoded-secret",
        Secret,
        Critical,
        "Possible hard-coded password or key",
        "Read secrets from the environment or a secret store, never from the source."
    ),
    rule!(
        "sql-concat",
        Vulnerability,
        Major,
        "SQL built by joining strings",
        "Use bound parameters so input can never become part of the query."
    ),
    rule!(
        "eval",
        Vulnerability,
        Critical,
        "Code is run from a string",
        "Avoid eval and exec: parse the data, or call the function directly."
    ),
    rule!(
        "shell-command",
        Vulnerability,
        Critical,
        "A shell command is run from a string",
        "Pass the program and its arguments as a list and skip the shell."
    ),
    rule!(
        "tls-verification-off",
        Vulnerability,
        Critical,
        "Certificate checks are turned off",
        "Keep TLS verification on; trust a private authority instead."
    ),
    rule!(
        "unsafe-c-function",
        Vulnerability,
        Major,
        "Unsafe C function {}",
        "Use a bounded version (snprintf, strncpy, fgets) and check the sizes."
    ),
    rule!(
        "html-injection",
        Vulnerability,
        Major,
        "HTML is written from a string",
        "Set textContent, or sanitise the markup before it reaches the page."
    ),
    rule!(
        "unsafe-deserialisation",
        Vulnerability,
        Major,
        "Data is loaded in a way that can run code",
        "Use yaml.safe_load, JSON or another format that cannot run code."
    ),
    rule!(
        "weak-hash",
        Hotspot,
        Major,
        "A weak hash function is used",
        "Use SHA-256 or better, and a password hash (argon2, bcrypt) for passwords."
    ),
    rule!(
        "insecure-url",
        Hotspot,
        Minor,
        "An http:// address sends data in the clear",
        "Use https:// unless this really is a local address."
    ),
    rule!(
        "unsafe-block",
        Hotspot,
        Major,
        "Unsafe code",
        "Wrap the unsafe part in a small safe function and say why it is sound."
    ),
    rule!(
        "may-panic",
        Bug,
        Major,
        "This can panic and stop the program",
        "Return an error with ? or handle the None and Err cases."
    ),
    rule!(
        "debug-output",
        Smell,
        Minor,
        "Debug output left in the code",
        "Remove it, or use the project's logging."
    ),
    rule!(
        "todo-comment",
        Smell,
        Info,
        "A comment promises work",
        "Do it now, or track it as an issue and remove the comment."
    ),
    rule!(
        "long-function",
        Smell,
        Minor,
        "Function {} is too long",
        "Split it into smaller functions that each do one thing."
    ),
    rule!(
        "complex-function",
        Smell,
        Minor,
        "Function {} is too complex",
        "Break the branches into smaller functions or simplify the conditions."
    ),
    rule!(
        "many-parameters",
        Smell,
        Minor,
        "Function {} takes too many parameters",
        "Group related parameters into a struct."
    ),
    rule!(
        "ndl-colour-literal",
        Convention,
        Major,
        "A colour is written in the code",
        "Take it from the theme tokens (look.palette) so every theme and contrast mode works."
    ),
    rule!(
        "ndl-untranslated-string",
        Convention,
        Major,
        "Interface text does not go through tr()",
        "Write the English through tr() or trf() and add its Spanish to the SPANISH table."
    ),
    rule!(
        "ndl-missing-spanish",
        Convention,
        Major,
        "Text without Spanish: {}",
        "Add the string and its translation to the sorted SPANISH table."
    ),
    rule!(
        "ndl-missing-spdx",
        Convention,
        Minor,
        "The file has no licence header",
        "Start the file with // SPDX-License-Identifier: MIT OR Apache-2.0."
    ),
    rule!(
        "duplicated-block",
        Smell,
        Minor,
        "This block is copied from {}",
        "Move the shared code into one function."
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_ids_are_unique() {
        for (i, a) in RULES.iter().enumerate() {
            assert!(
                RULES.iter().skip(i + 1).all(|b| b.id != a.id),
                "{} is twice",
                a.id
            );
        }
    }

    #[test]
    fn severity_is_ordered_worst_last() {
        assert!(Severity::Blocker > Severity::Critical);
        assert!(Severity::Major > Severity::Minor);
        assert!(Severity::Minor > Severity::Info);
    }
}
