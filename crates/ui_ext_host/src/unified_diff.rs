//! Applying a proposal's unified diff (`review:propose`, design wit-0.4 §B.10) to the file's
//! current text, strictly: every hunk at the line its header names, every context and removed
//! line exactly as in the file (a `\r` before the newline aside), hunks in order and apart. No
//! fuzz and no offset search: a diff made against another version of the file does not apply,
//! and the proposal is refused rather than landing somewhere the extension did not mean.

/// One `@@ -a,b +c,d @@` hunk: where it starts in the old text (1-based; for an empty old side
/// the line after which it inserts) and its lines.
struct Hunk<'a> {
    old_start: usize,
    old_count: usize,
    new_count: usize,
    lines: Vec<Line<'a>>,
}

enum Line<'a> {
    Context(&'a str),
    Removed(&'a str),
    /// The text, and whether a newline follows it.
    Added(&'a str, bool),
}

fn range(text: &str) -> Option<(usize, usize)> {
    match text.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

fn header(line: &str) -> Result<(usize, usize, usize), String> {
    let invalid = || format!("not a hunk header: {line:?}");
    let rest = line.strip_prefix("@@ -").ok_or_else(invalid)?;
    let (ranges, _) = rest.split_once(" @@").ok_or_else(invalid)?;
    let (old, new) = ranges.split_once(" +").ok_or_else(invalid)?;
    let (old_start, old_count) = range(old).ok_or_else(invalid)?;
    let (_, new_count) = range(new).ok_or_else(invalid)?;
    Ok((old_start, old_count, new_count))
}

fn parse(diff: &str) -> Result<Vec<Hunk<'_>>, String> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut lines = diff.split_inclusive('\n').peekable();
    while let Some(raw) = lines.next() {
        let line = raw.trim_end_matches('\n').trim_end_matches('\r');
        if line.starts_with("@@ ") {
            let (old_start, old_count, new_count) = header(line)?;
            hunks.push(Hunk {
                old_start,
                old_count,
                new_count,
                lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = hunks.last_mut() else {
            // `diff --git`, `index`, `---`, `+++` and anything else before the first hunk.
            continue;
        };
        let (old_seen, new_seen) = counts(&hunk.lines);
        let complete = old_seen == hunk.old_count && new_seen == hunk.new_count;
        if let Some(text) = line.strip_prefix('+').filter(|_| !complete) {
            let newline = !lines
                .peek()
                .is_some_and(|next| next.starts_with("\\ No newline"));
            hunk.lines.push(Line::Added(text, newline));
        } else if let Some(text) = line.strip_prefix('-').filter(|_| !complete) {
            hunk.lines.push(Line::Removed(text));
        } else if let Some(text) = line.strip_prefix(' ').filter(|_| !complete) {
            hunk.lines.push(Line::Context(text));
        } else if line.is_empty() && !complete {
            // Some tools drop the space of an empty context line.
            hunk.lines.push(Line::Context(""));
        } else if line.starts_with('\\') {
            // "\ No newline at end of file", taken with the line before it.
        } else if complete {
            // The next file's headers, or trailing text: only hunks of one file are applied.
            if line.starts_with("--- ") || line.starts_with("diff ") {
                return Err("the diff changes more than one file".into());
            }
        } else {
            return Err(format!("unexpected line in a hunk: {line:?}"));
        }
    }
    for hunk in &hunks {
        if counts(&hunk.lines) != (hunk.old_count, hunk.new_count) {
            return Err("a hunk is shorter than its header says".into());
        }
    }
    if hunks.is_empty() {
        return Err("the diff has no hunks".into());
    }
    Ok(hunks)
}

fn counts(lines: &[Line]) -> (usize, usize) {
    lines.iter().fold((0, 0), |(old, new), line| match line {
        Line::Context(_) => (old + 1, new + 1),
        Line::Removed(_) => (old + 1, new),
        Line::Added(..) => (old, new + 1),
    })
}

fn content(line: &str) -> &str {
    line.trim_end_matches('\n').trim_end_matches('\r')
}

/// `old` with `diff` applied. The added lines take the file's line ending (CRLF when the file
/// has one).
pub fn apply(old: &str, diff: &str) -> Result<String, String> {
    let hunks = parse(diff)?;
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let ending = if old.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = String::with_capacity(old.len() + diff.len());
    let mut next = 0;
    for hunk in hunks {
        // `-0,0` inserts before the first line; otherwise the hunk starts at line `old_start`.
        let start = if hunk.old_count == 0 {
            hunk.old_start
        } else {
            hunk.old_start
                .checked_sub(1)
                .ok_or("a hunk starts at line 0")?
        };
        if start < next {
            return Err("the hunks overlap or are out of order".into());
        }
        if start > old_lines.len() {
            return Err(format!(
                "a hunk starts at line {} of a file with {} lines",
                hunk.old_start,
                old_lines.len()
            ));
        }
        out.extend(old_lines[next..start].iter().copied());
        let mut at = start;
        for line in hunk.lines {
            match line {
                Line::Context(text) | Line::Removed(text) => {
                    let found = old_lines.get(at).copied();
                    if found.map(content) != Some(text) {
                        return Err(format!(
                            "line {} does not match the diff (the file changed since it was made)",
                            at + 1
                        ));
                    }
                    if let (Line::Context(_), Some(found)) = (&line, found) {
                        out.push_str(found);
                    }
                    at += 1;
                }
                Line::Added(text, newline) => {
                    out.push_str(text);
                    if newline {
                        out.push_str(ending);
                    }
                }
            }
        }
        next = at;
    }
    out.extend(old_lines[next..].iter().copied());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::apply;

    #[test]
    fn test_applies_hunks_in_place() {
        let old = "one\ntwo\nthree\nfour\nfive\n";
        let diff = "--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n-one\n+ONE\n two\n@@ -4,2 +4,3 @@\n four\n-five\n+FIVE\n+six\n";
        assert_eq!(
            apply(old, diff).unwrap(),
            "ONE\ntwo\nthree\nfour\nFIVE\nsix\n"
        );
    }

    #[test]
    fn test_insert_into_empty_file_and_missing_newline() {
        assert_eq!(apply("", "@@ -0,0 +1,2 @@\n+a\n+b\n").unwrap(), "a\nb\n");
        let diff =
            "@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+b\n\\ No newline at end of file\n";
        assert_eq!(apply("a", diff).unwrap(), "b");
    }

    #[test]
    fn test_keeps_crlf() {
        let old = "a\r\nb\r\n";
        assert_eq!(
            apply(old, "@@ -1,2 +1,2 @@\n a\n-b\n+c\n").unwrap(),
            "a\r\nc\r\n"
        );
    }

    #[test]
    fn test_refuses_what_does_not_apply() {
        let old = "one\ntwo\n";
        assert!(
            apply(old, "@@ -1,1 +1,1 @@\n-uno\n+ONE\n").is_err(),
            "wrong line"
        );
        assert!(
            apply(old, "@@ -2,1 +2,1 @@\n-one\n+ONE\n").is_err(),
            "no offset search"
        );
        assert!(
            apply(old, "@@ -5,1 +5,1 @@\n-x\n+y\n").is_err(),
            "past the end"
        );
        assert!(
            apply(old, "@@ -1,2 +1,2 @@\n-one\n+ONE\n").is_err(),
            "short hunk"
        );
        assert!(apply(old, "no hunks\n").is_err());
        assert!(
            apply(
                old,
                "@@ -2,1 +2,1 @@\n-two\n+2\n@@ -1,1 +1,1 @@\n-one\n+1\n"
            )
            .is_err(),
            "out of order"
        );
        assert!(
            apply(
                old,
                "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-one\n+1\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-a\n+b\n"
            )
            .is_err(),
            "two files"
        );
    }
}
