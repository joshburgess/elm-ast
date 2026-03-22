/// A position in source code (byte offset + line/column).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    /// Byte offset from the start of the source.
    pub offset: usize,
    /// 1-based line number.
    pub line: u32,
    /// 1-based column number (in UTF-8 bytes).
    pub column: u32,
}

/// A span in source code, defined by a start and end position.
///
/// Spans are half-open: they include the start position and exclude the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

impl Span {
    /// Create a new span from start and end positions.
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }

    /// Create a dummy span for synthesized nodes (not from source).
    pub fn dummy() -> Self {
        let pos = Position {
            offset: 0,
            line: 0,
            column: 0,
        };
        Self {
            start: pos,
            end: pos,
        }
    }

    /// Merge two spans into one that covers both.
    pub fn merge(self, other: Span) -> Span {
        let start = if self.start.offset <= other.start.offset {
            self.start
        } else {
            other.start
        };
        let end = if self.end.offset >= other.end.offset {
            self.end
        } else {
            other.end
        };
        Span { start, end }
    }
}
