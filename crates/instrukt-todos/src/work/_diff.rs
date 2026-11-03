//! A line diff between two retained artifact versions.

/// One line of a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    Same(String),
    Removed(String),
    Added(String),
}

/// The lines of `old` and `new` as a longest-common-subsequence diff:
/// unchanged lines, then removals before additions at each change.
pub(crate) fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // lcs[i][j]: the common subsequence length of a[i..] and b[j..].
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            out.push(DiffLine::Same(a[i].to_owned()));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(DiffLine::Removed(a[i].to_owned()));
            i += 1;
        } else {
            out.push(DiffLine::Added(b[j].to_owned()));
            j += 1;
        }
    }
    out.extend(a[i..].iter().map(|l| DiffLine::Removed((*l).to_owned())));
    out.extend(b[j..].iter().map(|l| DiffLine::Added((*l).to_owned())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_line_is_a_removal_and_an_addition_between_kept_lines() {
        let diff = diff_lines("a\nb\nc\n", "a\nB\nc\nd\n");
        assert_eq!(
            diff,
            vec![
                DiffLine::Same("a".into()),
                DiffLine::Removed("b".into()),
                DiffLine::Added("B".into()),
                DiffLine::Same("c".into()),
                DiffLine::Added("d".into()),
            ]
        );
    }

    #[test]
    fn identical_versions_differ_in_nothing() {
        assert!(
            diff_lines("x\ny", "x\ny")
                .iter()
                .all(|l| matches!(l, DiffLine::Same(_)))
        );
    }
}
