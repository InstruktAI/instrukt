//! The `yaml-language-server` modeline: the first-line comment that names a
//! YAML file's JSON Schema for the developer's editor
//! (`docs/project/spec/feature/publication/editor-schemas.md`, "Modelines").
//!
//! A modeline is a comment, so it never changes what a file means; these
//! functions only place it, keeping every other byte of the file.

/// What every modeline opens with.
pub const MODELINE_PREFIX: &str = "# yaml-language-server:";

/// The modeline naming `schema`, a path relative to the file's directory.
pub fn modeline(schema: &str) -> String {
    format!("{MODELINE_PREFIX} $schema={schema}")
}

/// The first line of `bytes`, without its line ending, when it is a
/// modeline.
pub fn leading(bytes: &[u8]) -> Option<&[u8]> {
    let line = first_line(bytes);
    line.starts_with(MODELINE_PREFIX.as_bytes())
        .then(|| trim_line_ending(line))
}

/// `bytes` starting with the modeline naming `schema`: a leading modeline
/// naming another path is replaced, a missing one is inserted as the first
/// line, and every other byte is kept. `None` when `bytes` already starts
/// with that modeline.
pub fn with_modeline(bytes: &[u8], schema: &str) -> Option<Vec<u8>> {
    let wanted = modeline(schema);
    let (rest, ending) = match leading(bytes) {
        Some(line) if line == wanted.as_bytes() => return None,
        Some(_) => {
            let line = first_line(bytes);
            let ending = &line[trim_line_ending(line).len()..];
            (
                &bytes[line.len()..],
                if ending.is_empty() {
                    b"\n".as_slice()
                } else {
                    ending
                },
            )
        }
        None => (bytes, b"\n".as_slice()),
    };
    let mut out = Vec::with_capacity(wanted.len() + ending.len() + rest.len());
    out.extend_from_slice(wanted.as_bytes());
    out.extend_from_slice(ending);
    out.extend_from_slice(rest);
    Some(out)
}

/// The first line of `bytes` with its line ending.
fn first_line(bytes: &[u8]) -> &[u8] {
    match bytes.iter().position(|b| *b == b'\n') {
        Some(at) => &bytes[..=at],
        None => bytes,
    }
}

fn trim_line_ending(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const SCHEMA: &str = "../.instrukt/schemas/project-index.schema.json";

    #[test]
    fn a_missing_modeline_is_inserted_as_the_first_line() {
        assert_eq!(
            with_modeline(b"expanded: []\n", SCHEMA).unwrap(),
            format!("{}\nexpanded: []\n", modeline(SCHEMA)).into_bytes()
        );
    }

    #[test]
    fn an_empty_file_gets_the_modeline_alone() {
        assert_eq!(
            with_modeline(b"", SCHEMA).unwrap(),
            format!("{}\n", modeline(SCHEMA)).into_bytes()
        );
    }

    #[test]
    fn a_modeline_naming_another_path_is_replaced_with_its_line_ending_kept() {
        let text = format!("{}\r\nlisted: []\r\n", modeline("old.json"));
        assert_eq!(
            with_modeline(text.as_bytes(), SCHEMA).unwrap(),
            format!("{}\r\nlisted: []\r\n", modeline(SCHEMA)).into_bytes()
        );
    }

    #[test]
    fn a_file_already_naming_its_schema_is_left_as_it_is() {
        let text = format!("{}\nlisted: []\n", modeline(SCHEMA));
        assert_eq!(with_modeline(text.as_bytes(), SCHEMA), None);
    }

    #[test]
    fn a_modeline_is_read_without_its_line_ending() {
        let text = format!("{}\r\nx: 1\n", modeline(SCHEMA));
        assert_eq!(leading(text.as_bytes()), Some(modeline(SCHEMA).as_bytes()));
        assert_eq!(leading(b"# a comment\n"), None);
    }

    proptest! {
        #[test]
        fn placing_the_modeline_keeps_every_other_byte(body in "[a-z: \\n#-]{0,40}") {
            prop_assume!(!body.starts_with(MODELINE_PREFIX));
            let placed = with_modeline(body.as_bytes(), SCHEMA).unwrap();
            let line = modeline(SCHEMA);
            prop_assert_eq!(&placed[line.len() + 1..], body.as_bytes());
            prop_assert_eq!(with_modeline(&placed, SCHEMA), None);
            let moved = with_modeline(&placed, "other.json").unwrap();
            prop_assert_eq!(&moved[modeline("other.json").len() + 1..], body.as_bytes());
        }
    }
}
