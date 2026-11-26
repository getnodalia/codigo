// SPDX-License-Identifier: MIT OR Apache-2.0
//! The structure rules: how long, how branching and how wide each function is, read from the
//! syntax tree.

use tree_sitter::{Node, Tree};

use crate::rules::{Finding, Severity};

const LONG: usize = 60;
const VERY_LONG: usize = 120;
const COMPLEX: usize = 12;
const VERY_COMPLEX: usize = 20;
const MAX_PARAMS: usize = 7;
/// Nodes visited per file at most, so a hostile tree cannot hold the analysis.
const MAX_NODES: usize = 2_000_000;

const FUNCTIONS: &[&str] = &[
    "function_item",
    "function_definition",
    "function_declaration",
    "method_definition",
    "method_declaration",
    "constructor_declaration",
    "arrow_function",
    "function_expression",
    "generator_function_declaration",
];

/// Node kinds that add a path through a function.
const BRANCHES: &[&str] = &[
    "if_expression",
    "if_statement",
    "if_let_expression",
    "elif_clause",
    "else_if_clause",
    "for_expression",
    "for_statement",
    "for_in_statement",
    "enhanced_for_statement",
    "while_expression",
    "while_statement",
    "while_let_expression",
    "do_statement",
    "loop_expression",
    // A match or switch counts once however many arms it has: a long table of cases is not
    // hard to follow the way nested decisions are.
    "match_expression",
    "switch_statement",
    "switch_expression",
    "select_statement",
    "case_statement",
    "catch_clause",
    "except_clause",
    "conditional_expression",
    "ternary_expression",
];

const BOOLEAN_NODES: &[&str] = &[
    "binary_expression",
    "boolean_operator",
    "logical_expression",
    "binary_operator",
];

pub(crate) fn check(tree: &Tree, text: &str, out: &mut Vec<Finding>) {
    let mut budget = MAX_NODES;
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if budget == 0 {
            return;
        }
        budget -= 1;
        if FUNCTIONS.contains(&node.kind()) {
            function(node, text, out, &mut budget);
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
}

fn name_of(node: Node, text: &str) -> String {
    node.child_by_field_name("name")
        .and_then(|n| text.get(n.byte_range()))
        .map_or_else(|| "(anonymous)".to_owned(), str::to_owned)
}

fn function(node: Node, text: &str, out: &mut Vec<Finding>, budget: &mut usize) {
    let (first, last) = (node.start_position().row, node.end_position().row);
    let name = name_of(node, text);
    // Anchor the finding on the name when there is one, else on the whole first line.
    let (line, start, end) = match node.child_by_field_name("name") {
        Some(n) => (
            n.start_position().row,
            n.start_position().column,
            n.end_position().column,
        ),
        None => (
            first,
            node.start_position().column,
            node.start_position().column + 1,
        ),
    };
    let lines = last.saturating_sub(first) + 1;
    if lines > LONG {
        let severity = if lines > VERY_LONG {
            Severity::Major
        } else {
            Severity::Minor
        };
        push(
            out,
            "long-function",
            line,
            start,
            end,
            &name,
            Some(severity),
        );
    }
    let complexity = 1 + branches(node, text, budget);
    if complexity > COMPLEX {
        let severity = if complexity > VERY_COMPLEX {
            Severity::Major
        } else {
            Severity::Minor
        };
        let f = Finding::new("complex-function", line, start, end).map(|f| {
            f.with_severity(severity)
                .with_detail(format!("{name} ({complexity})"))
        });
        out.extend(f);
    }
    if let Some(params) = node.child_by_field_name("parameters") {
        if params.named_child_count() > MAX_PARAMS {
            push(out, "many-parameters", line, start, end, &name, None);
        }
    }
}

fn push(
    out: &mut Vec<Finding>,
    rule: &str,
    line: usize,
    start: usize,
    end: usize,
    name: &str,
    severity: Option<Severity>,
) {
    if let Some(mut f) = Finding::new(rule, line, start, end) {
        f = f.with_detail(name);
        if let Some(s) = severity {
            f = f.with_severity(s);
        }
        out.push(f);
    }
}

/// Decision points inside a function, not counting nested functions: they get their own count.
fn branches(function: Node, text: &str, budget: &mut usize) -> usize {
    let mut count = 0;
    let mut stack = Vec::new();
    let mut cursor = function.walk();
    stack.extend(function.children(&mut cursor));
    while let Some(node) = stack.pop() {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let kind = node.kind();
        if FUNCTIONS.contains(&kind) {
            continue;
        }
        if BRANCHES.contains(&kind) {
            count += 1;
        } else if BOOLEAN_NODES.contains(&kind) {
            let op = node
                .child_by_field_name("operator")
                .and_then(|o| text.get(o.byte_range()));
            if matches!(op, Some("&&" | "||" | "and" | "or")) {
                count += 1;
            }
        }
        let mut c = node.walk();
        stack.extend(node.children(&mut c));
    }
    count
}

#[cfg(test)]
mod tests {
    use crate::analyze_text;
    use codigo_syntax::find;

    fn rules_for(lang: &str, text: &str) -> Vec<(&'static str, Option<String>)> {
        let lang = find(lang).expect("language");
        analyze_text(lang, "src/x", text)
            .into_iter()
            .map(|f| (f.rule.id, f.detail))
            .collect()
    }

    #[test]
    fn a_long_function_is_found_with_its_name() {
        let body = "    let x = 1;\n".repeat(70);
        let found = rules_for("rust", &format!("fn big() {{\n{body}}}\n"));
        assert!(found.contains(&("long-function", Some("big".to_owned()))));
    }

    #[test]
    fn branching_is_counted_and_nested_functions_stand_alone() {
        let mut src = String::from("fn busy(a: i32) -> i32 {\n");
        for i in 0..14 {
            src.push_str(&format!(
                "    if a > {i} && a < {} {{ return {i}; }}\n",
                i + 9
            ));
        }
        src.push_str("    0\n}\nfn calm() { if true { let _ = 1; } }\n");
        let found = rules_for("rust", &src);
        let complex: Vec<_> = found.iter().filter(|f| f.0 == "complex-function").collect();
        assert_eq!(complex.len(), 1);
        assert!(complex[0]
            .1
            .as_deref()
            .is_some_and(|d| d.starts_with("busy (")));
    }

    #[test]
    fn many_parameters_are_found_in_python() {
        let found = rules_for("python", "def f(a, b, c, d, e, f, g, h):\n    return a\n");
        assert!(found.iter().any(|f| f.0 == "many-parameters"));
    }

    #[test]
    fn small_functions_are_quiet() {
        assert!(rules_for("rust", "fn ok(a: i32) -> i32 { a + 1 }\n").is_empty());
    }
}
