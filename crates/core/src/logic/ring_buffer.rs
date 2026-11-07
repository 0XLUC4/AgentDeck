//! Keeps the most recent `cap` bytes of terminal output, stored as chunks (`src/lib/ringBuffer.ts`).
//! Used to repaint a terminal on remount/detach and for the orchestrator's `read_output`.

use std::collections::VecDeque;

/// 1 MB per pane, like the 512K UTF-16 chars of the original.
pub const DEFAULT_CAP: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct RingBuffer {
    chunks: VecDeque<String>,
    size: usize,
    cap: usize,
}

impl Default for RingBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_CAP)
    }
}

/// Last `n` bytes of `text`, moved forward to a char boundary.
fn tail_of(text: &str, n: usize) -> &str {
    let mut start = text.len().saturating_sub(n);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

impl RingBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            chunks: VecDeque::new(),
            size: 0,
            cap,
        }
    }

    pub fn push(&mut self, data: &str) {
        self.chunks.push_back(data.to_owned());
        self.size += data.len();
        while self.size > self.cap && self.chunks.len() > 1 {
            if let Some(old) = self.chunks.pop_front() {
                self.size -= old.len();
            }
        }
        if self.size > self.cap {
            // A single chunk larger than cap: keep its tail.
            let kept = tail_of(&self.text(), self.cap).to_owned();
            self.size = kept.len();
            self.chunks = VecDeque::from([kept]);
        }
    }

    pub fn text(&self) -> String {
        self.chunks.iter().map(String::as_str).collect()
    }

    /// Last `n` bytes, joining only the chunks needed.
    pub fn tail(&self, n: usize) -> String {
        let mut length = 0;
        let mut from = self.chunks.len();
        while from > 0 && length < n {
            from -= 1;
            length += self.chunks[from].len();
        }
        let joined: String = self.chunks.range(from..).map(String::as_str).collect();
        tail_of(&joined, n).to_owned()
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.size = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_under_cap_and_drops_oldest() {
        let mut buf = RingBuffer::new(10);
        buf.push("abc");
        buf.push("def");
        assert_eq!(buf.text(), "abcdef");
        let mut buf = RingBuffer::new(5);
        buf.push("abc");
        buf.push("de");
        buf.push("fg");
        assert_eq!(buf.text(), "defg");
    }

    #[test]
    fn oversized_chunk_keeps_tail() {
        let mut buf = RingBuffer::new(3);
        buf.push("abcdef");
        assert_eq!(buf.text(), "def");
        buf.push("g");
        assert_eq!(buf.text(), "g");
    }

    #[test]
    fn many_chunks_and_tail() {
        let mut buf = RingBuffer::new(4);
        for c in "abcdefghij".repeat(300).chars() {
            buf.push(&c.to_string());
        }
        assert_eq!(buf.text(), "ghij");
        assert_eq!(buf.tail(2), "ij");
        assert_eq!(buf.tail(10), "ghij");
        buf.clear();
        assert_eq!(buf.text(), "");
    }

    #[test]
    fn tail_respects_char_boundaries() {
        let mut buf = RingBuffer::new(100);
        buf.push("a\u{e9}");
        assert_eq!(buf.tail(1), "");
        assert_eq!(buf.tail(2), "\u{e9}");
    }
}
