//! VS Code style line commands, as pure functions over the note text.
//! Offsets are UTF-8 byte offsets; `sel` is the current selection.

use std::ops::Range;

/// Replace `range` with `text`, then select `select`.
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
    pub select: Range<usize>,
}

fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(text: &str, pos: usize) -> usize {
    text[pos..].find('\n').map_or(text.len(), |i| pos + i)
}

/// The whole lines touched by the selection, without the final newline.
fn block(text: &str, sel: &Range<usize>) -> Range<usize> {
    // A selection that ends at the start of a line does not include that line.
    let end = if sel.end > sel.start && text[..sel.end].ends_with('\n') {
        sel.end - 1
    } else {
        sel.end
    };
    line_start(text, sel.start)..line_end(text, end)
}

fn shift(sel: &Range<usize>, by: isize) -> Range<usize> {
    (sel.start as isize + by) as usize..(sel.end as isize + by) as usize
}

pub fn move_lines_up(text: &str, sel: Range<usize>) -> Option<Edit> {
    let b = block(text, &sel);
    if b.start == 0 {
        return None;
    }
    let prev = line_start(text, b.start - 1)..b.start - 1;
    Some(Edit {
        range: prev.start..b.end,
        text: format!("{}\n{}", &text[b.clone()], &text[prev.clone()]),
        select: shift(&sel, -(prev.len() as isize + 1)),
    })
}

pub fn move_lines_down(text: &str, sel: Range<usize>) -> Option<Edit> {
    let b = block(text, &sel);
    if b.end == text.len() {
        return None;
    }
    let next = b.end + 1..line_end(text, b.end + 1);
    Some(Edit {
        range: b.start..next.end,
        text: format!("{}\n{}", &text[next.clone()], &text[b.clone()]),
        select: shift(&sel, next.len() as isize + 1),
    })
}

/// Duplicate the selected lines. The cursor stays on the upper copy when
/// `up` is set and moves to the lower copy otherwise.
pub fn copy_lines(text: &str, sel: Range<usize>, up: bool) -> Edit {
    let b = block(text, &sel);
    let copy = format!("\n{}", &text[b.clone()]);
    let select = if up { sel } else { shift(&sel, copy.len() as isize) };
    Edit {
        range: b.end..b.end,
        text: copy,
        select,
    }
}

pub fn delete_lines(text: &str, sel: Range<usize>) -> Edit {
    let b = block(text, &sel);
    let (range, cursor) = if b.end < text.len() {
        (b.start..b.end + 1, b.start)
    } else if b.start > 0 {
        (b.start - 1..b.end, line_start(text, b.start - 1))
    } else {
        (b.clone(), 0)
    };
    Edit {
        range,
        text: String::new(),
        select: cursor..cursor,
    }
}

/// The selected lines including the trailing newline (Ctrl+L).
pub fn select_lines(text: &str, sel: Range<usize>) -> Range<usize> {
    let b = block(text, &sel);
    b.start..(b.end + 1).min(text.len())
}

pub fn insert_line(text: &str, sel: Range<usize>, above: bool) -> Edit {
    let b = block(text, &sel);
    let (at, cursor) = if above { (b.start, b.start) } else { (b.end, b.end + 1) };
    Edit {
        range: at..at,
        text: "\n".to_string(),
        select: cursor..cursor,
    }
}

/// Ctrl+D: with nothing selected, the word under the cursor; otherwise the
/// next occurrence of the selected text, wrapping around.
pub fn next_occurrence(text: &str, sel: Range<usize>) -> Option<Range<usize>> {
    if sel.is_empty() {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let start = text[..sel.start]
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_word(*c))
            .last()
            .map_or(sel.start, |(i, _)| i);
        let end = text[sel.start..]
            .char_indices()
            .find(|(_, c)| !is_word(*c))
            .map_or(text.len(), |(i, _)| sel.start + i);
        return (start < end).then_some(start..end);
    }
    let needle = &text[sel.clone()];
    let found = text[sel.end..]
        .find(needle)
        .map(|i| sel.end + i)
        .or_else(|| text[..sel.start].find(needle))?;
    Some(found..found + needle.len())
}
