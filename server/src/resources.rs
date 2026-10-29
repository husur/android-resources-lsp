use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use lsp_types::Range;
use roxmltree::{Document, Node};

use crate::lines::Lines;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    String,
    Plurals,
}

#[derive(Debug, PartialEq)]
pub enum Value {
    Text(String),
    /// `(quantity, text)` for each item, in order.
    Plurals(Vec<(String, String)>),
}

#[derive(Debug, PartialEq)]
pub struct Resource {
    pub name: String,
    pub value: Value,
    /// The value of the `name` attribute.
    pub range: Range,
}

impl Resource {
    pub fn kind(&self) -> Kind {
        match self.value {
            Value::Text(_) => Kind::String,
            Value::Plurals(_) => Kind::Plurals,
        }
    }
}

/// The strings and plurals of a `res/values` XML file, with their text as the app shows it.
pub fn parse(xml: &str) -> Vec<Resource> {
    let Ok(document) = Document::parse(xml) else {
        return vec![];
    };
    let lines = Lines::new(xml);
    document
        .root_element()
        .children()
        .filter_map(|element| {
            let name = element.attribute_node("name")?;
            let value = match element.tag_name().name() {
                "string" => Value::Text(text(element)),
                "plurals" => Value::Plurals(
                    element
                        .children()
                        .filter(|item| item.has_tag_name("item"))
                        .filter_map(|item| Some((item.attribute("quantity")?.into(), text(item))))
                        .collect(),
                ),
                _ => return None,
            };
            Some(Resource {
                name: name.value().to_string(),
                value,
                range: lines.range(name.range_value()),
            })
        })
        .collect()
}

/// Applies Android's escapes, quotes and whitespace collapsing, and keeps the text of inner tags
/// such as `<xliff:g>`.
fn text(element: Node) -> String {
    let raw: String = element
        .descendants()
        .filter(|node| node.is_text())
        .filter_map(|node| node.text())
        .collect();
    let mut text = String::new();
    let mut space = false;
    let mut quoted = false;
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        let c = match c {
            '"' => {
                quoted = !quoted;
                continue;
            }
            c if c.is_whitespace() && !quoted => {
                space = true;
                continue;
            }
            '\\' => match chars.next() {
                Some('n') => '\n',
                Some('t') => '\t',
                Some(escaped) => escaped,
                None => break,
            },
            c => c,
        };
        if space && !text.is_empty() {
            text.push(' ');
        }
        space = false;
        text.push(c);
    }
    text
}

/// The XML files of the `res/values` directories under `root`, build outputs and hidden
/// directories aside.
pub fn scan(root: &Path) -> Vec<PathBuf> {
    let mut files = vec![];
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory).into_iter().flatten().flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() && !skipped(&entry.file_name()) => directories.push(path),
                Ok(kind) if kind.is_file() && is_values(&path) => files.push(path),
                _ => {}
            }
        }
    }
    files.sort();
    files
}

/// Whether `path`, relative to the root it was found in, is a file that [`scan`] returns.
pub fn indexed(path: &Path) -> bool {
    let mut directories = path.parent().into_iter().flat_map(Path::iter);
    is_values(path) && !directories.any(skipped)
}

fn is_values(path: &Path) -> bool {
    let mut directories = path.iter().rev().skip(1);
    path.extension() == Some(OsStr::new("xml"))
        && directories.next() == Some(OsStr::new("values"))
        && directories.next() == Some(OsStr::new("res"))
}

fn skipped(directory: &OsStr) -> bool {
    directory == "build" || directory.to_string_lossy().starts_with('.')
}
