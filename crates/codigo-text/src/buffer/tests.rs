// SPDX-License-Identifier: MIT OR Apache-2.0
use super::*;

fn buf(text: &str) -> Buffer {
    Buffer::from_text(text)
}

fn at(b: &mut Buffer, idx: usize) {
    b.set_cursor(idx);
}

fn carets(b: &Buffer) -> Vec<usize> {
    b.selections().iter().map(|s| s.head).collect()
}

#[test]
fn typing_replaces_selections_and_coalesces_for_undo() {
    let mut b = buf("hello");
    at(&mut b, 5);
    for c in " world".chars() {
        b.type_char(c, Pairs::NONE);
    }
    assert_eq!(b.text(), "hello world");
    b.undo();
    assert_eq!(
        b.text(),
        "hello",
        "one undo takes back the whole run of typing"
    );
    assert!(!b.can_undo());
    b.redo();
    assert_eq!(b.text(), "hello world");
    assert_eq!(carets(&b), vec![11]);
}

#[test]
fn undo_restores_the_cursors_that_were_there() {
    let mut b = buf("abc");
    b.set_selections(Selections::single(Selection::new(1, 3)));
    b.insert_text("X");
    assert_eq!(b.text(), "aX");
    b.undo();
    assert_eq!(b.text(), "abc");
    assert_eq!(b.primary().range(), 1..3);
}

#[test]
fn several_cursors_edit_together() {
    let mut b = buf("a\nb\nc");
    b.set_selections(Selections::from_vec(
        vec![
            Selection::caret(1),
            Selection::caret(3),
            Selection::caret(5),
        ],
        0,
    ));
    b.insert_text("!");
    assert_eq!(b.text(), "a!\nb!\nc!");
    assert_eq!(carets(&b), vec![2, 5, 8]);
    b.delete_backward(Pairs::NONE);
    assert_eq!(b.text(), "a\nb\nc");
}

#[test]
fn pairs_close_overtype_and_delete_together() {
    let mut b = buf("");
    b.type_char('(', Pairs::DEFAULT);
    assert_eq!(b.text(), "()");
    assert_eq!(carets(&b), vec![1]);
    b.type_char(')', Pairs::DEFAULT);
    assert_eq!(b.text(), "()", "typing the closer steps over it");
    assert_eq!(carets(&b), vec![2]);
    b.undo();
    let mut b = buf("");
    b.type_char('[', Pairs::DEFAULT);
    b.delete_backward(Pairs::DEFAULT);
    assert_eq!(b.text(), "", "backspace between a pair removes both");
}

#[test]
fn a_quote_after_a_word_is_not_doubled() {
    let mut b = buf("don");
    at(&mut b, 3);
    b.type_char('\'', Pairs::DEFAULT);
    assert_eq!(b.text(), "don'");
}

#[test]
fn typing_an_opener_over_a_selection_surrounds_it() {
    let mut b = buf("word");
    b.set_selections(Selections::single(Selection::new(0, 4)));
    b.type_char('"', Pairs::DEFAULT);
    assert_eq!(b.text(), "\"word\"");
}

#[test]
fn newline_keeps_the_indent_and_opens_blocks() {
    let mut b = buf("    if x {");
    at(&mut b, 10);
    b.newline(IndentRules::default());
    assert_eq!(b.text(), "    if x {\n        ");
    let mut b = buf("fn f() {}");
    at(&mut b, 8);
    b.newline(IndentRules::default());
    assert_eq!(b.text(), "fn f() {\n    \n}");
    assert_eq!(b.line_col(carets(&b)[0]), (1, 4));
    let mut b = buf("def f():");
    at(&mut b, 8);
    b.newline(IndentRules {
        open: "",
        colon: true,
    });
    assert_eq!(b.text(), "def f():\n    ");
}

#[test]
fn backspace_in_indentation_removes_one_level() {
    let mut b = buf("        x");
    at(&mut b, 8);
    b.delete_backward(Pairs::NONE);
    assert_eq!(b.text(), "    x");
    b.delete_backward(Pairs::NONE);
    assert_eq!(b.text(), "x");
}

#[test]
fn deleting_words_and_line_ends() {
    let mut b = buf("foo bar baz");
    at(&mut b, 7);
    b.delete_word_backward();
    assert_eq!(b.text(), "foo  baz");
    b.delete_word_forward();
    assert_eq!(b.text(), "foo ");
    let mut b = buf("one\ntwo");
    at(&mut b, 1);
    b.delete_to_line_end();
    assert_eq!(b.text(), "o\ntwo");
    b.delete_to_line_end();
    assert_eq!(
        b.text(),
        "otwo",
        "at the end of a line it takes the newline"
    );
}

#[test]
fn tab_indents_to_the_next_stop_and_blocks_indent_whole_lines() {
    let mut b = buf("ab\ncd");
    at(&mut b, 2);
    b.indent();
    assert_eq!(b.text(), "ab  \ncd");
    let mut b = buf("a\nb\nc");
    b.set_selections(Selections::single(Selection::new(0, 3)));
    b.indent();
    assert_eq!(b.text(), "    a\n    b\nc");
    assert_eq!(
        b.primary().range(),
        0..11,
        "the selection still covers the lines"
    );
    b.outdent();
    assert_eq!(b.text(), "a\nb\nc");
}

#[test]
fn outdent_takes_a_tab_or_up_to_one_level_of_spaces() {
    let mut b = buf("\tx\n      y\n  z");
    b.set_selections(Selection::new(0, 14).into_selections());
    b.outdent();
    assert_eq!(b.text(), "x\n  y\nz");
}

#[test]
fn lines_move_up_and_down() {
    let mut b = buf("a\nb\nc");
    at(&mut b, 2);
    b.move_lines(true);
    assert_eq!(b.text(), "b\na\nc");
    assert_eq!(carets(&b), vec![0]);
    b.move_lines(false);
    b.move_lines(false);
    assert_eq!(b.text(), "a\nc\nb");
    assert!(
        b.move_lines(false).is_empty(),
        "the last line cannot go lower"
    );
    at(&mut b, 0);
    assert!(b.move_lines(true).is_empty());
}

#[test]
fn a_block_of_lines_moves_together() {
    let mut b = buf("1\n2\n3\n4");
    b.set_selections(Selection::new(2, 5).into_selections());
    b.move_lines(true);
    assert_eq!(b.text(), "2\n3\n1\n4");
    b.undo();
    assert_eq!(b.text(), "1\n2\n3\n4");
}

#[test]
fn lines_duplicate_and_delete() {
    let mut b = buf("a\nb");
    at(&mut b, 0);
    b.duplicate_lines(false);
    assert_eq!(b.text(), "a\na\nb");
    assert_eq!(carets(&b), vec![2], "the caret goes to the copy below");
    b.duplicate_lines(true);
    assert_eq!(b.text(), "a\na\na\nb");
    let mut b = buf("x\ny\nz");
    at(&mut b, 2);
    b.delete_lines();
    assert_eq!(b.text(), "x\nz");
    assert_eq!(carets(&b), vec![2]);
    let mut b = buf("x\ny");
    at(&mut b, 2);
    b.delete_lines();
    assert_eq!(b.text(), "x", "no empty line is left at the end");
    let mut b = buf("only");
    b.delete_lines();
    assert_eq!(b.text(), "");
}

#[test]
fn joining_and_opening_lines() {
    let mut b = buf("a\n   b\nc");
    at(&mut b, 0);
    b.join_lines();
    assert_eq!(b.text(), "a b\nc");
    let mut b = buf("  a");
    at(&mut b, 3);
    b.open_line(true);
    assert_eq!(b.text(), "  a\n  ");
    at(&mut b, 3);
    b.open_line(false);
    assert_eq!(b.text(), "  \n  a\n  ");
}

#[test]
fn comments_toggle_on_and_off() {
    let mut b = buf("    a\n\n    b");
    b.select_all();
    b.toggle_line_comment("//");
    assert_eq!(b.text(), "    // a\n\n    // b");
    b.toggle_line_comment("//");
    assert_eq!(b.text(), "    a\n\n    b");
    let mut b = buf("x\n// y");
    b.select_all();
    b.toggle_line_comment("//");
    assert_eq!(
        b.text(),
        "// x\n// // y",
        "a mixed block is commented first"
    );
}

#[test]
fn vertical_moves_keep_the_goal_column() {
    let mut b = buf("long line\nab\nlong line");
    at(&mut b, 8);
    b.move_cursors(Motion::Down, false);
    assert_eq!(carets(&b), vec![12], "end of the short line");
    b.move_cursors(Motion::Down, false);
    assert_eq!(
        b.line_col(carets(&b)[0]),
        (2, 8),
        "the goal column came back"
    );
    b.move_cursors(Motion::Down, false);
    assert_eq!(
        carets(&b),
        vec![b.len_chars()],
        "past the last line goes to the end"
    );
    b.move_cursors(Motion::DocStart, false);
    b.move_cursors(Motion::Up, false);
    assert_eq!(carets(&b), vec![0]);
}

#[test]
fn home_goes_to_the_text_then_to_the_start() {
    let mut b = buf("   abc");
    at(&mut b, 5);
    b.move_cursors(Motion::LineStart, false);
    assert_eq!(carets(&b), vec![3]);
    b.move_cursors(Motion::LineStart, false);
    assert_eq!(carets(&b), vec![0]);
    b.move_cursors(Motion::LineEnd, false);
    assert_eq!(carets(&b), vec![6]);
}

#[test]
fn left_and_right_collapse_a_selection_to_its_edges() {
    let mut b = buf("abcdef");
    b.set_selections(Selection::new(1, 4).into_selections());
    b.move_cursors(Motion::Left, false);
    assert_eq!(carets(&b), vec![1]);
    b.set_selections(Selection::new(1, 4).into_selections());
    b.move_cursors(Motion::Right, false);
    assert_eq!(carets(&b), vec![4]);
    b.set_selections(Selection::new(1, 4).into_selections());
    b.move_cursors(Motion::Right, true);
    assert_eq!(b.primary().range(), 1..5);
}

#[test]
fn paragraphs_move_between_blank_lines() {
    let mut b = buf("a\nb\n\nc\nd");
    at(&mut b, 0);
    b.move_cursors(Motion::ParagraphDown, false);
    assert_eq!(b.line_col(carets(&b)[0]), (2, 0));
    b.move_cursors(Motion::ParagraphDown, false);
    assert_eq!(carets(&b), vec![b.len_chars()]);
    b.move_cursors(Motion::ParagraphUp, false);
    assert_eq!(b.line_col(carets(&b)[0]), (2, 0));
}

#[test]
fn cursors_are_added_above_and_below() {
    let mut b = buf("abc\nabc\nabc");
    at(&mut b, 5);
    b.add_cursor_vertical(false);
    b.add_cursor_vertical(true);
    assert_eq!(carets(&b), vec![1, 5, 9]);
    b.insert_text("-");
    assert_eq!(b.text(), "a-bc\na-bc\na-bc");
    b.collapse_selections();
    assert_eq!(b.selections().len(), 1);
}

#[test]
fn ctrl_d_selects_the_word_then_each_occurrence() {
    let mut b = buf("foo bar foo baz foo");
    at(&mut b, 1);
    b.select_next_occurrence();
    assert_eq!(b.primary().range(), 0..3);
    b.select_next_occurrence();
    b.select_next_occurrence();
    assert_eq!(b.selections().len(), 3);
    b.select_next_occurrence();
    assert_eq!(b.selections().len(), 3, "no fourth one");
    b.insert_text("X");
    assert_eq!(b.text(), "X bar X baz X");
}

#[test]
fn select_all_occurrences() {
    let mut b = buf("a b a c a");
    at(&mut b, 0);
    b.select_all_occurrences();
    assert_eq!(b.selections().len(), 3);
}

#[test]
fn selecting_lines_and_words() {
    let mut b = buf("one\ntwo\nthree");
    b.select_line_at(5);
    assert_eq!(b.primary().range(), 4..8);
    b.select_word_at(9);
    assert_eq!(b.slice(b.primary().range()), "three");
    at(&mut b, 5);
    b.select_lines();
    assert_eq!(b.slice(b.primary().range()), "two\n");
}

#[test]
fn copy_takes_the_line_when_nothing_is_selected() {
    let mut b = buf("one\ntwo");
    at(&mut b, 5);
    assert_eq!(b.selected_text(true), "two\n");
    assert_eq!(b.selected_text(false), "");
    b.set_selections(Selection::new(0, 3).into_selections());
    assert_eq!(b.selected_text(true), "one");
}

#[test]
fn pasting_a_whole_line_goes_above_the_caret_line() {
    let mut b = buf("a\nbc");
    at(&mut b, 3);
    b.paste("X\n");
    assert_eq!(b.text(), "a\nX\nbc");
    assert_eq!(
        b.line_col(carets(&b)[0]),
        (2, 1),
        "the caret stays in its column"
    );
}

#[test]
fn pasting_one_line_per_cursor() {
    let mut b = buf("\n\n");
    b.set_selections(Selections::from_vec(
        vec![
            Selection::caret(0),
            Selection::caret(1),
            Selection::caret(2),
        ],
        0,
    ));
    b.paste("a\nb\nc");
    assert_eq!(b.text(), "a\nb\nc");
}

#[test]
fn pasting_crlf_text_leaves_no_carriage_returns() {
    let mut b = buf("");
    b.paste("a\r\nb");
    assert_eq!(b.text(), "a\nb");
}

#[test]
fn dirty_follows_the_saved_state_through_undo_and_redo() {
    let mut b = buf("x");
    assert!(!b.is_dirty());
    at(&mut b, 1);
    b.insert_text("y");
    assert!(b.is_dirty());
    b.mark_saved();
    assert!(!b.is_dirty());
    b.insert_text("z");
    assert!(
        b.is_dirty(),
        "typing after a save is not merged into the saved state"
    );
    b.undo();
    assert!(!b.is_dirty());
    b.undo();
    assert!(b.is_dirty());
    b.redo();
    assert!(!b.is_dirty());
}

#[test]
fn external_edits_move_the_cursors_and_undo() {
    let mut b = buf("hello world");
    at(&mut b, 11);
    let changes = b
        .apply_edits(vec![Edit::new(6..11, "there"), Edit::new(0..5, "Hi")])
        .unwrap();
    assert_eq!(b.text(), "Hi there");
    assert_eq!(changes.len(), 2);
    assert_eq!(carets(&b), vec![8]);
    b.undo();
    assert_eq!(b.text(), "hello world");
    assert!(b.apply_edits(vec![Edit::new(0..99, "")]).is_none());
}

#[test]
fn replacing_all_text_is_one_undo_step() {
    let mut b = buf("one\ntwo");
    at(&mut b, 5);
    b.replace_all_text("one\ntwo\nthree");
    assert_eq!(carets(&b), vec![5]);
    assert!(b.replace_all_text("one\ntwo\nthree").is_empty());
    b.undo();
    assert_eq!(b.text(), "one\ntwo");
}

#[test]
fn edits_that_overlap_are_merged_not_lost() {
    let mut b = buf("abc");
    b.set_selections(Selections::from_vec(
        vec![Selection::caret(1), Selection::caret(2)],
        0,
    ));
    b.delete_backward(Pairs::NONE);
    assert_eq!(b.text(), "c");
}

#[test]
fn never_panics_on_odd_input() {
    let mut b = buf("");
    b.delete_backward(Pairs::DEFAULT);
    b.delete_forward();
    b.delete_lines();
    b.move_lines(true);
    b.move_lines(false);
    b.join_lines();
    b.outdent();
    b.indent();
    b.undo();
    b.redo();
    b.set_cursor(999);
    assert_eq!(carets(&b), vec![b.len_chars()]);
    b.move_cursors(Motion::PageDown(10), false);
    b.select_next_occurrence();
    b.select_all_occurrences();
    b.toggle_line_comment("#");
    let mut b = buf("é🙂\n\n\t");
    for m in [
        Motion::Left,
        Motion::Right,
        Motion::Up,
        Motion::Down,
        Motion::WordLeft,
        Motion::WordRight,
        Motion::LineStart,
        Motion::LineEnd,
        Motion::DocEnd,
        Motion::ParagraphUp,
        Motion::PageUp(5),
    ] {
        b.move_cursors(m, true);
    }
    b.newline(IndentRules::default());
}

trait IntoSelections {
    fn into_selections(self) -> Selections;
}

impl IntoSelections for Selection {
    fn into_selections(self) -> Selections {
        Selections::single(self)
    }
}
