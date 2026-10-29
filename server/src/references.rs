use std::ops;

use lsp_types::Range;

use crate::lines::Lines;
use crate::resources::Kind;

#[derive(Debug, PartialEq)]
pub struct Reference {
    pub kind: Kind,
    pub name: String,
    /// The name only, without `R.string.`.
    pub range: Range,
    /// `R` comes with its package, as in `com.example.R.string.title`, and may be a library's.
    pub qualified: bool,
}

/// The `R.string.*` and `R.plurals.*` references of a Kotlin source, in order. Those of
/// `android.R`, the platform's resources, are left out, and so is commented-out code, which often
/// names strings deleted since.
pub fn find(text: &str) -> Vec<Reference> {
    let lines = Lines::new(text);
    let comments = comments(text);
    let mut references = vec![];
    for (kind, prefix) in [(Kind::String, "R.string."), (Kind::Plurals, "R.plurals.")] {
        for (start, _) in text.match_indices(prefix) {
            let before = &text[..start];
            let commented = comments.iter().any(|comment| comment.contains(&start));
            if before.ends_with(is_identifier) || platform(before) || commented {
                continue;
            }
            let name_start = start + prefix.len();
            let name_end = text[name_start..]
                .find(|c| !is_identifier(c))
                .map_or(text.len(), |length| name_start + length);
            if name_start == name_end {
                continue;
            }
            references.push(Reference {
                kind,
                name: text[name_start..name_end].to_string(),
                range: lines.range(name_start..name_end),
                qualified: before.ends_with('.'),
            });
        }
    }
    references.sort_by_key(|reference| reference.range.start);
    references
}

/// Strings and characters are skipped, so that the `//` of a URL starts no comment. A quote inside
/// a string template can throw this off until the end of the line.
fn comments(text: &str) -> Vec<ops::Range<usize>> {
    let bytes = text.as_bytes();
    let mut comments = vec![];
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        match &bytes[at..] {
            [b'/', b'/', ..] => {
                at = text[at..].find('\n').map_or(bytes.len(), |end| at + end);
                comments.push(start..at);
            }
            [b'/', b'*', ..] => {
                // Kotlin comments nest.
                let mut depth = 0;
                while at < bytes.len() {
                    match &bytes[at..] {
                        [b'/', b'*', ..] => depth += 1,
                        [b'*', b'/', ..] => depth -= 1,
                        _ => {
                            at += 1;
                            continue;
                        }
                    }
                    at += 2;
                    if depth == 0 {
                        break;
                    }
                }
                comments.push(start..at);
            }
            [b'"', b'"', b'"', ..] => {
                at = text[at + 3..]
                    .find("\"\"\"")
                    .map_or(bytes.len(), |end| at + 3 + end + 3);
            }
            [quote @ (b'"' | b'\''), ..] => {
                at += 1;
                while at < bytes.len() && bytes[at] != *quote && bytes[at] != b'\n' {
                    at += if bytes[at] == b'\\' { 2 } else { 1 };
                }
                at += 1;
            }
            _ => at += 1,
        }
    }
    comments
}

fn platform(before: &str) -> bool {
    before
        .strip_suffix("android.")
        .is_some_and(|package| !package.ends_with(|c| is_identifier(c) || c == '.'))
}

fn is_identifier(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
