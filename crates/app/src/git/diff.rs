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

/// A review remark on one diff line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub path: String,
    /// Index of the line in the shown diff.
    pub index: usize,
    /// Line number in the file (the old file for a deleted line).
    pub line: Option<u32>,
    /// The diff line, marker included.
    pub code: String,
    pub text: String,
}

/// File line number of each diff line: the new file's, or the old file's for a deleted line.
pub fn line_numbers(lines: &[&str], kinds: &[DiffLineKind]) -> Vec<Option<u32>> {
    let (mut old, mut new) = (0u32, 0u32);
    lines
        .iter()
        .zip(kinds)
        .map(|(line, kind)| match kind {
            DiffLineKind::Hunk => {
                // "@@ -12,3 +14,4 @@": where each side starts.
                let start = |sign: char| -> Option<u32> {
                    line.split(' ').find_map(|part| part.strip_prefix(sign)?.split(',').next()?.parse().ok())
                };
                (old, new) = (start('-').unwrap_or(0), start('+').unwrap_or(0));
                None
            }
            DiffLineKind::Meta => None,
            DiffLineKind::Del => {
                old += 1;
                Some(old - 1)
            }
            DiffLineKind::Add => {
                new += 1;
                Some(new - 1)
            }
            DiffLineKind::Context => {
                old += 1;
                new += 1;
                Some(new - 1)
            }
        })
        .collect()
}

/// One prompt carrying every remark, for the agent that wrote the changes.
pub fn review_prompt(comments: &[Comment]) -> String {
    let mut prompt =
        String::from("Review remarks on your changes. Address each one, then summarize what you changed.\n");
    for c in comments {
        let place = c.line.map_or_else(|| c.path.clone(), |n| format!("{}:{n}", c.path));
        let code: String = c.code.trim().chars().take(120).collect();
        prompt.push_str(&format!("\n- {place} `{code}`\n  {}\n", c.text));
    }
    prompt
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
    fn numbers_lines_and_builds_the_review_prompt() {
        let diff = ["diff --git a/a b/a", "@@ -10,3 +20,3 @@ fn x()", " same", "-old", "+new", " end"];
        let numbers = line_numbers(&diff, &classify_diff(&diff));
        assert_eq!(numbers, [None, None, Some(20), Some(11), Some(21), Some(22)]);
        let remark =
            Comment { path: "src/a.rs".into(), index: 4, line: Some(21), code: "+new".into(), text: "Use a constant".into() };
        assert!(review_prompt(&[remark]).ends_with("\n- src/a.rs:21 `+new`\n  Use a constant\n"));
    }

    #[test]
    fn splits_lines_without_the_trailing_newline() {
        assert_eq!(diff_lines("a\nb\n"), ["a", "b"]);
        assert_eq!(diff_lines("a\n\n"), ["a", ""]);
        assert!(diff_lines("").is_empty());
    }
}
