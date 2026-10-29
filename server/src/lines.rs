use std::ops;

use lsp_types::{Position, Range};

/// Converts byte offsets in a text to LSP positions, whose characters count UTF-16 code units.
pub struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    pub fn new(text: &'a str) -> Self {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(newline, _)| newline + 1))
            .collect();
        Self { text, starts }
    }

    pub fn position(&self, offset: usize) -> Position {
        let line = self.starts.partition_point(|&start| start <= offset) - 1;
        let character = self.text[self.starts[line]..offset].encode_utf16().count();
        Position::new(line as u32, character as u32)
    }

    pub fn range(&self, span: ops::Range<usize>) -> Range {
        Range::new(self.position(span.start), self.position(span.end))
    }
}
