//! Unified diff line classification for the Git panel's diff viewer (original: components/Git/diff.ts).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Add,
    Del,
    Hunk,
    Meta,
    Context,
}

/// Kind of each line of `git diff` output. Inside a hunk, "--- x" is a deleted line, not a file header.
pub fn classify_diff(lines: &[&str]) -> Vec<DiffLineKind> {
    let mut in_hunk = false;
    lines
        .iter()
        .map(|line| {
            if line.starts_with("@@") {
                in_hunk = true;
                return DiffLineKind::Hunk;
            }
            if line.starts_with("diff ") {
                in_hunk = false;
            }
            if !in_hunk {
                return DiffLineKind::Meta; // diff --git, index, ---/+++ headers, Binary files ...
            }
            match line.as_bytes().first() {
                Some(b'+') => DiffLineKind::Add,
                Some(b'-') => DiffLineKind::Del,
                Some(b'\\') => DiffLineKind::Meta, // \ No newline at end of file
                _ => DiffLineKind::Context,
            }
        })
        .collect()
}

/// Lines of a diff, without the trailing empty line git's final newline produces.
pub fn diff_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.strip_suffix('\n').unwrap_or(text).split('\n').collect()
}

#[cfg(test)]
mod tests {
    use super::DiffLineKind::*;
    use super::*;

    #[test]
    fn classifies_every_kind_of_unified_diff_line() {
        let diff = [
            "diff --git a/a.txt b/a.txt",
            "index 5626abf..f719efd 100644",
            "--- a/a.txt",
            "+++ b/a.txt",
            "@@ -1,3 +1,3 @@",
            " same",
            "-old",
            "+new",
            "--- deleted line that started with two dashes",
            "+++ added line that started with two pluses",
            "\\ No newline at end of file",
            "diff --git a/b.png b/b.png",
            "--- a/b.png",
            "Binary files differ",
        ];
        assert_eq!(
            classify_diff(&diff),
            [Meta, Meta, Meta, Meta, Hunk, Context, Del, Add, Del, Add, Meta, Meta, Meta, Meta]
        );
    }

    #[test]
    fn splits_lines_without_the_trailing_newline() {
        assert_eq!(diff_lines("a\nb\n"), ["a", "b"]);
        assert_eq!(diff_lines("a\n\n"), ["a", ""]);
        assert!(diff_lines("").is_empty());
    }
}
