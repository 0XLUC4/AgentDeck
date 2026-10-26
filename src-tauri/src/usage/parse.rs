//! Incremental parsers for agent transcripts: Claude Code (`~/.claude/projects/**.jsonl`)
//! and Codex (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl[.zst]`).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    /// Uncached input.
    pub input: u64,
    pub output: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// UTC, milliseconds.
    pub ts: i64,
    pub model: Arc<str>,
    pub cwd: Arc<str>,
    pub tokens: Tokens,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Claude,
    Codex,
}

/// Parsed state of one transcript file; `feed` only sees lines appended since the last call.
#[derive(Debug)]
pub struct FileState {
    pub source: Source,
    pub offset: u64,
    pub len: u64,
    pub mtime: i64,
    pub entries: Vec<Entry>,
    /// Claude writes one line per content block, each repeating the message's usage: keep the last one.
    claude_ids: HashMap<String, usize>,
    codex_total: Option<Tokens>,
    model: Arc<str>,
    cwd: Arc<str>,
}

impl FileState {
    pub fn new(source: Source) -> Self {
        Self {
            source,
            offset: 0,
            len: 0,
            mtime: 0,
            entries: Vec::new(),
            claude_ids: HashMap::new(),
            codex_total: None,
            model: Arc::from(""),
            cwd: Arc::from(""),
        }
    }

    /// Parses lines appended since the last call. A truncated/rewritten file is reparsed from scratch;
    /// `.zst` archives are immutable and parsed once.
    pub fn refresh(&mut self, path: &Path, len: u64, mtime: i64) -> std::io::Result<()> {
        if len == self.len && mtime == self.mtime {
            return Ok(());
        }
        let zst = path.extension().is_some_and(|e| e == "zst");
        if len < self.offset || zst {
            *self = Self::new(self.source);
        }
        let mut file = std::fs::File::open(path)?;
        if zst {
            let decoder = ruzstd::decoding::StreamingDecoder::new(BufReader::new(file))
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            self.feed(BufReader::new(decoder))?;
        } else {
            file.seek(SeekFrom::Start(self.offset))?;
            self.feed(BufReader::with_capacity(256 * 1024, file))?;
        }
        self.len = len;
        self.mtime = mtime;
        Ok(())
    }

    /// Feeds complete lines only: a partially written last line waits for the next refresh.
    pub fn feed(&mut self, mut reader: impl BufRead) -> std::io::Result<()> {
        let mut line = Vec::new();
        loop {
            line.clear();
            let n = reader.read_until(b'\n', &mut line)?;
            if n == 0 || line.last() != Some(&b'\n') {
                return Ok(());
            }
            self.offset += n as u64;
            self.line(&line);
        }
    }

    fn line(&mut self, line: &[u8]) {
        match self.source {
            Source::Claude if contains(line, b"\"usage\"") => self.claude_line(line),
            Source::Codex
                if contains(line, b"token_count")
                    || contains(line, b"\"turn_context\"")
                    || contains(line, b"\"session_meta\"") =>
            {
                self.codex_line(line)
            }
            _ => {}
        }
    }

    fn claude_line(&mut self, line: &[u8]) {
        let Ok(rec) = serde_json::from_slice::<ClaudeLine>(line) else {
            return;
        };
        let Some(msg) = rec.message.filter(|_| rec.kind == "assistant") else {
            return;
        };
        let Some(u) = msg.usage else { return };
        let (w5, w1h) = match u.cache_creation {
            Some(c) => (c.ephemeral_5m_input_tokens, c.ephemeral_1h_input_tokens),
            None => (u.cache_creation_input_tokens, 0),
        };
        let entry = Entry {
            ts: parse_ts(&rec.timestamp),
            model: self.intern_model(msg.model.as_deref().unwrap_or("")),
            cwd: self.intern_cwd(rec.cwd.as_deref().unwrap_or("")),
            tokens: Tokens {
                input: u.input_tokens,
                output: u.output_tokens,
                cache_write_5m: w5,
                cache_write_1h: w1h,
                cache_read: u.cache_read_input_tokens,
            },
        };
        match msg.id {
            Some(id) => match self.claude_ids.get(&id) {
                Some(&i) => self.entries[i] = entry,
                None => {
                    self.claude_ids.insert(id, self.entries.len());
                    self.entries.push(entry);
                }
            },
            None => self.entries.push(entry),
        }
    }

    fn codex_line(&mut self, line: &[u8]) {
        let Ok(rec) = serde_json::from_slice::<CodexLine>(line) else {
            return;
        };
        let p = &rec.payload;
        match rec.kind.as_str() {
            "session_meta" | "turn_context" => {
                if let Some(cwd) = p.get("cwd").and_then(Value::as_str) {
                    self.cwd = self.intern_cwd(cwd);
                }
                if let Some(model) = p.get("model").and_then(Value::as_str) {
                    self.model = self.intern_model(model);
                }
            }
            "event_msg" if p.get("type").and_then(Value::as_str) == Some("token_count") => {
                let Some(total) = p.pointer("/info/total_token_usage") else {
                    return;
                };
                let field = |k: &str| total.get(k).and_then(Value::as_u64).unwrap_or(0);
                let cached = field("cached_input_tokens");
                let now = Tokens {
                    input: field("input_tokens").saturating_sub(cached),
                    output: field("output_tokens"),
                    cache_write_5m: field("cache_write_input_tokens"),
                    cache_write_1h: 0,
                    cache_read: cached,
                };
                let delta = match self.codex_total {
                    Some(prev) if now == prev => return, // repeated event
                    Some(prev) if covers(&now, &prev) => sub(&now, &prev),
                    _ => now, // first event, or a counter reset
                };
                self.codex_total = Some(now);
                self.entries.push(Entry {
                    ts: parse_ts(&rec.timestamp),
                    model: self.model.clone(),
                    cwd: self.cwd.clone(),
                    tokens: delta,
                });
            }
            _ => {}
        }
    }

    fn intern_model(&mut self, model: &str) -> Arc<str> {
        if &*self.model != model {
            self.model = Arc::from(model);
        }
        self.model.clone()
    }

    fn intern_cwd(&mut self, cwd: &str) -> Arc<str> {
        if &*self.cwd != cwd {
            self.cwd = Arc::from(cwd);
        }
        self.cwd.clone()
    }
}

fn covers(a: &Tokens, b: &Tokens) -> bool {
    a.input >= b.input
        && a.output >= b.output
        && a.cache_read >= b.cache_read
        && a.cache_write_5m >= b.cache_write_5m
}

fn sub(a: &Tokens, b: &Tokens) -> Tokens {
    Tokens {
        input: a.input - b.input,
        output: a.output - b.output,
        cache_write_5m: a.cache_write_5m - b.cache_write_5m,
        cache_write_1h: 0,
        cache_read: a.cache_read - b.cache_read,
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn parse_ts(ts: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(ts).map_or(0, |t| t.timestamp_millis())
}

#[derive(Deserialize)]
struct ClaudeLine {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    timestamp: String,
    cwd: Option<String>,
    message: Option<ClaudeMessage>,
}

#[derive(Deserialize)]
struct ClaudeMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<ClaudeUsage>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation: Option<CacheCreation>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CacheCreation {
    ephemeral_5m_input_tokens: u64,
    ephemeral_1h_input_tokens: u64,
}

#[derive(Deserialize)]
struct CodexLine {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    payload: Value,
}

/// Reads a whole reader into a fresh state (tests, one-shot use).
#[cfg(test)]
pub fn parse_all(source: Source, reader: impl std::io::Read) -> FileState {
    let mut state = FileState::new(source);
    state.feed(BufReader::new(reader)).unwrap();
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = include_str!("fixtures/claude.jsonl");
    const CODEX: &str = include_str!("fixtures/codex.jsonl");

    #[test]
    fn claude_dedupes_content_blocks_and_reads_cache_split() {
        let s = parse_all(Source::Claude, CLAUDE.as_bytes());
        assert_eq!(
            s.entries.len(),
            2,
            "same message id counted once, user lines ignored"
        );
        let a = &s.entries[0];
        assert_eq!(&*a.model, "claude-opus-5-5");
        assert_eq!(&*a.cwd, "D:\\proj");
        assert_eq!(
            a.tokens,
            Tokens {
                input: 2,
                output: 169,
                cache_write_5m: 0,
                cache_write_1h: 43073,
                cache_read: 30377
            }
        );
        assert_eq!(a.ts, 1_793_021_518_000, "last block line wins");
        // No cache_creation breakdown: all writes are 5 min.
        assert_eq!(s.entries[1].tokens.cache_write_5m, 100);
        assert_eq!(s.entries[1].tokens.output, 50, "later block line wins");
    }

    #[test]
    fn claude_waits_for_complete_last_line() {
        let cut = CLAUDE.trim_end_matches('\n');
        let s = parse_all(Source::Claude, cut.as_bytes());
        assert_eq!(s.offset as usize, cut.rfind('\n').unwrap() + 1);
    }

    #[test]
    fn codex_uses_total_deltas() {
        let s = parse_all(Source::Codex, CODEX.as_bytes());
        // Events: total 1000/200 in (cached 200) -> repeated -> 1500 -> reset to 300.
        assert_eq!(s.entries.len(), 3);
        assert_eq!(&*s.entries[0].model, "gpt-5-codex");
        assert_eq!(&*s.entries[0].cwd, "C:\\work\\app");
        assert_eq!(s.entries[0].tokens.input, 800);
        assert_eq!(s.entries[0].tokens.cache_read, 200);
        assert_eq!(s.entries[1].tokens.input, 400);
        assert_eq!(s.entries[1].tokens.cache_read, 100);
        assert_eq!(s.entries[1].tokens.output, 30);
        assert_eq!(s.entries[2].tokens.input, 300);
        assert_eq!(&*s.entries[2].model, "gpt-6-astra");
    }

    #[test]
    fn incremental_feed_matches_full_parse() {
        let (head, tail) = CODEX.split_at(CODEX.len() / 2);
        let mut s = FileState::new(Source::Codex);
        s.feed(BufReader::new(head.as_bytes())).unwrap();
        let rest = &CODEX[s.offset as usize..];
        assert!(rest.len() >= tail.len());
        s.feed(BufReader::new(rest.as_bytes())).unwrap();
        let full = parse_all(Source::Codex, CODEX.as_bytes());
        assert_eq!(s.entries.len(), full.entries.len());
        assert_eq!(s.offset, CODEX.len() as u64);
    }
}
