//! Rewriting a layer document so that it reads as a new value while every
//! byte outside the top-level keys whose values changed stays as it was:
//! comments, blank lines, key order, quoting and the modeline.
//!
//! The YAML library reads and writes values only, so this edits the text of
//! a block-style top-level mapping directly and keeps the result only when
//! the library reads it back as exactly the wanted value. A document of
//! another shape is left to the caller's whole rendering.

use serde_yaml_ng::{Mapping, Value};

use super::_render::render_yaml;

/// One top-level key's lines, `start..end` of the document's lines. Blank
/// lines and column-0 comments after it are not part of it: they lead the
/// next key.
struct Block {
    key: String,
    start: usize,
    end: usize,
}

/// `before` edited so that it reads as `after`: a top-level key whose value
/// is unchanged keeps its lines, a removed key loses its lines, a changed
/// scalar on its key's one line is replaced where it stands (a trailing
/// comment kept), any other changed key's lines are rendered anew and an
/// added key is appended. `None` when `before` is not a block-style mapping
/// of plain keys, or the edit would not read back as `after`.
pub(super) fn edit_in_place(before: &[u8], after: &Value) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(before).ok()?;
    let old = match serde_yaml_ng::from_str::<Value>(text).ok()? {
        Value::Mapping(map) => map,
        Value::Null => Mapping::new(),
        _ => return None,
    };
    let new = after.as_mapping()?;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let blocks = blocks(&lines)?;
    if blocks.len() != old.len() || blocks.iter().any(|b| old.get(b.key.as_str()).is_none()) {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let first = blocks.first().map_or(lines.len(), |b| b.start);
    out.extend(lines[..first].iter().copied());
    for (index, block) in blocks.iter().enumerate() {
        let next = blocks.get(index + 1).map_or(lines.len(), |b| b.start);
        let own = &lines[block.start..block.end];
        match new.get(block.key.as_str()) {
            None => {}
            Some(value) if old.get(block.key.as_str()) == Some(value) => {
                out.extend(own.iter().copied());
            }
            Some(value) => out.push_str(&replaced(own, &block.key, value)?),
        }
        out.extend(lines[block.end..next].iter().copied());
    }
    for (key, value) in new {
        if old.get(key).is_none() {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            let mut single = Mapping::new();
            single.insert(key.clone(), value.clone());
            out.push_str(&render_yaml(&Value::Mapping(single)).ok()?);
        }
    }
    let read = match serde_yaml_ng::from_str::<Value>(&out).ok()? {
        Value::Null => Value::Mapping(Mapping::new()),
        value => value,
    };
    (read == *after).then(|| out.into_bytes())
}

/// The top-level key blocks of `lines`, in order; `None` when a column-0
/// line is neither a plain key, a comment, a blank line nor a sequence item
/// continuing the key above it.
fn blocks(lines: &[&str]) -> Option<Vec<Block>> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if is_blank(line) || line.starts_with('#') {
            index += 1;
            continue;
        }
        let key = plain_key(line)?;
        if blocks.iter().any(|b| b.key == key) {
            return None;
        }
        let mut end = index + 1;
        while end < lines.len() && continues(lines[end]) {
            end += 1;
        }
        while end > index + 1 && is_blank(lines[end - 1]) {
            end -= 1;
        }
        blocks.push(Block {
            key,
            start: index,
            end,
        });
        index = end;
    }
    Some(blocks)
}

/// Whether `line` belongs to the key above it: indented, blank, or a
/// column-0 sequence item.
fn continues(line: &str) -> bool {
    line.starts_with([' ', '\t'])
        || is_blank(line)
        || (line.starts_with('-') && !line.starts_with("---"))
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// The plain key a column-0 `key:` line opens; `None` for anything else.
fn plain_key(line: &str) -> Option<String> {
    let (key, rest) = line.split_once(':')?;
    let plain = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !key.starts_with('-');
    let separated = rest.is_empty() || rest.starts_with([' ', '\t', '\r', '\n']);
    (plain && separated).then(|| key.to_owned())
}

/// The lines of `key` holding `value` instead: a scalar on the key's one
/// line is replaced where it stands, keeping a trailing comment and the
/// line ending; anything else is rendered anew.
fn replaced(own: &[&str], key: &str, value: &Value) -> Option<String> {
    let scalar = !matches!(value, Value::Mapping(_) | Value::Sequence(_));
    if let ([line], true) = (own, scalar) {
        let rendered = render_yaml(value).ok()?;
        let rendered = rendered.trim_end_matches('\n');
        if !rendered.contains('\n') {
            let body = line.trim_end_matches(['\r', '\n']);
            let ending = &line[body.len()..];
            let rest = &body[key.len() + 1..];
            let comment = rest.find(" #").map_or("", |at| &rest[at..]);
            return Some(format!("{key}: {rendered}{comment}{ending}"));
        }
    }
    let mut single = Mapping::new();
    single.insert(Value::String(key.to_owned()), value.clone());
    render_yaml(&Value::Mapping(single)).ok()
}

#[cfg(test)]
mod tests {
    use serde_yaml_ng::Value;

    use super::edit_in_place;

    fn yaml(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    /// `before` edited to read as `after`, checked to read back as `after`.
    fn edited(before: &str, after: &str) -> String {
        let bytes = edit_in_place(before.as_bytes(), &yaml(after)).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(yaml(&text), yaml(after), "the edit reads as the new value");
        text
    }

    /// The lines of `text` whose index `keep` admits.
    fn lines_where(text: &str, keep: impl Fn(usize) -> bool) -> Vec<&str> {
        text.lines()
            .enumerate()
            .filter(|(index, _)| keep(*index))
            .map(|(_, line)| line)
            .collect()
    }

    #[test]
    fn a_changed_scalar_is_replaced_where_it_stands_and_every_other_line_is_kept() {
        let before =
            "# mine\nschema_version: 3 # stamped\n\n# who approves\napprovals: {prd: peer}\n";
        let after = edited(before, "schema_version: 5\napprovals: {prd: peer}\n");
        let at = before
            .lines()
            .position(|line| line.starts_with("schema_version:"))
            .unwrap();
        assert_eq!(
            lines_where(&after, |i| i != at),
            lines_where(before, |i| i != at)
        );
        let comment = |text: &str| {
            text.lines()
                .nth(at)
                .unwrap()
                .split_once(" #")
                .unwrap()
                .1
                .to_owned()
        };
        assert_eq!(
            comment(&after),
            comment(before),
            "its trailing comment is kept"
        );
    }

    #[test]
    fn a_removed_key_loses_only_its_own_lines() {
        let before = "project: p\nactions:\n  test: make test\n\n# kept\napprovals: {prd: peer}\n";
        let after = edited(before, "project: p\napprovals: {prd: peer}\n");
        let block = before
            .lines()
            .position(|line| line.starts_with("actions:"))
            .unwrap();
        assert_eq!(
            after.lines().collect::<Vec<_>>(),
            lines_where(before, |i| i != block && i != block + 1)
        );
    }

    #[test]
    fn a_changed_mapping_is_rendered_anew_and_its_neighbours_are_kept() {
        let before = "# top\nagents:\n  order: [a]\nticks: off # mine\n";
        let after = edited(before, "agents: {roles: {}}\nticks: 'off'\n");
        assert_eq!(after.lines().next(), before.lines().next());
        assert_eq!(after.lines().last(), before.lines().last());
    }

    #[test]
    fn an_added_key_is_appended_after_the_kept_document() {
        let before = "project: p # mine\n";
        let after = edited(before, "project: p\nboard: acme/1\n");
        assert_eq!(&after.as_bytes()[..before.len()], before.as_bytes());
    }

    #[test]
    fn a_flow_document_is_left_to_whole_rendering() {
        assert_eq!(
            edit_in_place(b"{project: p}\n", &yaml("project: q\n")),
            None
        );
    }
}
