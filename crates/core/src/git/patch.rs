//! Hunk and line staging: from a one-file unified diff, a patch that keeps only some of its
//! changed lines, for `git apply --cached` (stage, the diff is index -> working tree) or
//! `git apply --cached --reverse` (unstage, the diff is HEAD -> index). Hunk headers are recounted.

use std::collections::BTreeSet;

struct HunkHeader<'a> {
    old_start: u32,
    old_count: u32,
    new_start: u32,
    new_count: u32,
    /// Text after the closing `@@` (the function name git shows), leading space included.
    tail: &'a str,
}

/// `12,3` or `12` (count 1).
fn parse_range(range: &str) -> Option<(u32, u32)> {
    let (start, count) = range.split_once(',').unwrap_or((range, "1"));
    Some((start.parse().ok()?, count.parse().ok()?))
}

/// `@@ -12,3 +14,4 @@ fn x()`.
fn parse_hunk(line: &str) -> Option<HunkHeader<'_>> {
    let (ranges, tail) = line.strip_prefix("@@ -")?.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let ((old_start, old_count), (new_start, new_count)) = (parse_range(old)?, parse_range(new)?);
    Some(HunkHeader { old_start, old_count, new_start, new_count, tail })
}

fn is_hunk(line: &str) -> bool {
    line.starts_with("@@")
}

fn is_change(line: &str) -> bool {
    line.starts_with('+') || line.starts_with('-')
}

/// End (exclusive) of the hunk whose header is at `header`.
fn hunk_end(lines: &[&str], header: usize) -> usize {
    (header + 1..lines.len()).find(|&i| is_hunk(lines[i])).unwrap_or(lines.len())
}

/// Indices of the added and deleted lines of the hunk whose header is at `header`.
pub fn hunk_changes(lines: &[&str], header: usize) -> Vec<usize> {
    if !lines.get(header).is_some_and(|l| is_hunk(l)) {
        return Vec::new();
    }
    (header + 1..hunk_end(lines, header)).filter(|&i| is_change(lines[i])).collect()
}

/// File header lines. A partial patch can't create (unstaging) nor delete (staging) the whole
/// file: it then becomes a plain modification.
fn file_header(lines: &[&str], partial: bool, reverse: bool) -> String {
    let whole = if reverse { "new file mode" } else { "deleted file mode" };
    let convert = partial && lines.iter().any(|l| l.starts_with(whole));
    let path_after = |prefix: &str| lines.iter().find_map(|l| l.strip_prefix(prefix)).unwrap_or_default();
    let mut out = String::new();
    for &line in lines {
        let line = match line {
            l if convert && (l.starts_with(whole) || l.starts_with("index ")) => continue,
            "--- /dev/null" if convert => format!("--- a/{}", path_after("+++ b/")),
            "+++ /dev/null" if convert => format!("+++ b/{}", path_after("--- a/")),
            l => l.to_owned(),
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Patch of a one-file diff (`lines`, as `git diff` prints them) keeping only the `selected`
/// added/deleted lines; `None` when none is selected or the diff has no hunk (binary file).
///
/// Staging (`reverse` false) applies to the index, the diff's old side: an unselected deleted
/// line stays as context, an unselected added line is dropped. Unstaging (`reverse` true) applies
/// in reverse to the index, the diff's new side: the other way round.
pub fn select_patch(lines: &[&str], selected: &BTreeSet<usize>, reverse: bool) -> Option<String> {
    let first = lines.iter().position(|l| is_hunk(l))?;
    let changes: Vec<usize> = (first..lines.len()).filter(|&i| is_change(lines[i])).collect();
    let picked = changes.iter().filter(|i| selected.contains(i)).count();
    if picked == 0 {
        return None;
    }
    let mut out = file_header(&lines[..first], picked < changes.len(), reverse);
    // The side the patch applies to keeps its line numbers; the other one shifts by `offset`,
    // the (new - old) line count of the hunks emitted so far.
    let kept_sign = if reverse { '+' } else { '-' };
    let mut offset: i64 = 0;
    let mut header = first;
    while header < lines.len() {
        let hunk = parse_hunk(lines[header])?;
        let end = hunk_end(lines, header);
        let mut body: Vec<String> = Vec::new();
        let (mut old, mut new, mut changed) = (0u32, 0u32, false);
        // "\ No newline at end of file" follows its line: dropped with it.
        let mut last_kept = true;
        for (i, &line) in lines.iter().enumerate().take(end).skip(header + 1) {
            let sign = line.chars().next().unwrap_or(' ');
            last_kept = match sign {
                '\\' => {
                    if last_kept {
                        body.push(line.to_owned());
                    }
                    continue;
                }
                '+' | '-' if selected.contains(&i) => {
                    if sign == '+' { new += 1 } else { old += 1 }
                    changed = true;
                    body.push(line.to_owned());
                    true
                }
                '+' | '-' if sign == kept_sign => {
                    old += 1;
                    new += 1;
                    body.push(format!(" {}", &line[1..]));
                    true
                }
                '+' | '-' => false,
                _ => {
                    old += 1;
                    new += 1;
                    body.push(if line.is_empty() { " ".to_owned() } else { line.to_owned() });
                    true
                }
            };
        }
        if changed {
            // Unified format: a range of zero lines names the line before it.
            let (start, count) = if reverse { (hunk.new_start, hunk.new_count) } else { (hunk.old_start, hunk.old_count) };
            let before = i64::from(if count == 0 { start } else { start.saturating_sub(1) });
            let (old_before, new_before) = if reverse { (before - offset, before) } else { (before, before + offset) };
            let start_of = |before: i64, count: u32| if count == 0 { before } else { before + 1 };
            out.push_str(&format!(
                "@@ -{},{old} +{},{new} @@{}\n",
                start_of(old_before, old),
                start_of(new_before, new),
                hunk.tail
            ));
            for line in body {
                out.push_str(&line);
                out.push('\n');
            }
            offset += i64::from(new) - i64::from(old);
        }
        header = end;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_HUNKS: [&str; 15] = [
        "diff --git a/f b/f",
        "index 1111111..2222222 100644",
        "--- a/f",
        "+++ b/f",
        "@@ -1,4 +1,4 @@ ctx",
        " a",
        "-b",
        "+B",
        " c",
        " d",
        "@@ -10,2 +10,4 @@",
        " x",
        "+y",
        "+z",
        " w",
    ];
    const HEADER: &str = "diff --git a/f b/f\nindex 1111111..2222222 100644\n--- a/f\n+++ b/f\n";

    fn set(indices: &[usize]) -> BTreeSet<usize> {
        indices.iter().copied().collect()
    }

    #[test]
    fn lists_the_changes_of_a_hunk() {
        assert_eq!(hunk_changes(&TWO_HUNKS, 4), [6, 7]);
        assert_eq!(hunk_changes(&TWO_HUNKS, 10), [12, 13]);
        assert!(hunk_changes(&TWO_HUNKS, 5).is_empty());
        assert!(hunk_changes(&TWO_HUNKS, 99).is_empty());
    }

    #[test]
    fn nothing_selected_or_no_hunk_gives_no_patch() {
        assert_eq!(select_patch(&TWO_HUNKS, &set(&[]), false), None);
        assert_eq!(select_patch(&TWO_HUNKS, &set(&[5]), false), None); // a context line
        let binary = ["diff --git a/i.png b/i.png", "Binary files a/i.png and b/i.png differ"];
        assert_eq!(select_patch(&binary, &set(&[1]), false), None);
    }

    #[test]
    fn stages_whole_hunks_unchanged() {
        let patch = select_patch(&TWO_HUNKS, &set(&hunk_changes(&TWO_HUNKS, 4)), false).unwrap();
        assert_eq!(patch, format!("{HEADER}@@ -1,4 +1,4 @@ ctx\n a\n-b\n+B\n c\n d\n"));
    }

    #[test]
    fn stages_lines_and_recounts_headers() {
        // Only +z: the first hunk has nothing left and is skipped.
        let patch = select_patch(&TWO_HUNKS, &set(&[13]), false).unwrap();
        assert_eq!(patch, format!("{HEADER}@@ -10,2 +10,3 @@\n x\n+z\n w\n"));
        // -b without +B shifts the second hunk's new start by one.
        let patch = select_patch(&TWO_HUNKS, &set(&[6, 12]), false).unwrap();
        assert_eq!(patch, format!("{HEADER}@@ -1,4 +1,3 @@ ctx\n a\n-b\n c\n d\n@@ -10,2 +9,3 @@\n x\n+y\n w\n"));
        // +B without -b: the deleted line stays as context.
        let patch = select_patch(&TWO_HUNKS, &set(&[7]), false).unwrap();
        assert_eq!(patch, format!("{HEADER}@@ -1,4 +1,5 @@ ctx\n a\n b\n+B\n c\n d\n"));
    }

    #[test]
    fn unstages_lines_against_the_new_side() {
        // Unselected deleted lines are dropped, unselected added ones become context.
        let patch = select_patch(&TWO_HUNKS, &set(&[7, 13]), true).unwrap();
        assert_eq!(
            patch,
            format!("{HEADER}@@ -1,3 +1,4 @@ ctx\n a\n+B\n c\n d\n@@ -9,3 +10,4 @@\n x\n y\n+z\n w\n")
        );
    }

    #[test]
    fn new_and_deleted_files() {
        let new_file = [
            "diff --git a/n b/n",
            "new file mode 100644",
            "index 0000000..abcdef0",
            "--- /dev/null",
            "+++ b/n",
            "@@ -0,0 +1,3 @@",
            "+one",
            "+two",
            "+three",
        ];
        // Staging part of an untracked file creates it with those lines.
        let patch = select_patch(&new_file, &set(&[6, 8]), false).unwrap();
        assert!(patch.starts_with("diff --git a/n b/n\nnew file mode 100644\n"));
        assert!(patch.ends_with("--- /dev/null\n+++ b/n\n@@ -0,0 +1,2 @@\n+one\n+three\n"));
        // Unstaging part of a new file keeps it in the index: a modification patch.
        let patch = select_patch(&new_file, &set(&[7]), true).unwrap();
        assert_eq!(patch, "diff --git a/n b/n\n--- a/n\n+++ b/n\n@@ -1,2 +1,3 @@\n one\n+two\n three\n");
        // Unstaging all of it removes it: header unchanged.
        let patch = select_patch(&new_file, &set(&[6, 7, 8]), true).unwrap();
        assert!(patch.contains("new file mode 100644\n") && patch.contains("@@ -0,0 +1,3 @@\n"));

        let deleted = [
            "diff --git a/d b/d",
            "deleted file mode 100644",
            "index abcdef0..0000000",
            "--- a/d",
            "+++ /dev/null",
            "@@ -1,2 +0,0 @@",
            "-one",
            "-two",
        ];
        // Staging part of a deletion keeps the file in the index.
        let patch = select_patch(&deleted, &set(&[6]), false).unwrap();
        assert_eq!(patch, "diff --git a/d b/d\n--- a/d\n+++ b/d\n@@ -1,2 +1,1 @@\n-one\n two\n");
    }

    #[test]
    fn no_newline_marker_follows_its_line() {
        let diff = [
            "--- a/f",
            "+++ b/f",
            "@@ -1 +1 @@",
            "-old",
            "\\ No newline at end of file",
            "+new",
            "\\ No newline at end of file",
        ];
        let patch = select_patch(&diff, &set(&[3]), false).unwrap();
        assert_eq!(patch, "--- a/f\n+++ b/f\n@@ -1,1 +0,0 @@\n-old\n\\ No newline at end of file\n");
    }
}
