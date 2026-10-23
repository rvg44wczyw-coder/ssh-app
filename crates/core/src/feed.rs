//! The live feed as events: the turns of a session file kept on the phone and updated by chunks
//! of new lines instead of being rebuilt from the whole file. Pure state, no I/O.
//!
//! The rule that matters: feeding a file in any number of chunks, cut anywhere (also in the middle
//! of a line), gives exactly the turns of `parse_transcript_jsonl` of the whole file.

use crate::transcript::{TranscriptParser, TranscriptTurn};
use std::sync::Mutex;

/// A chunk longer than this is refused (the host sends events of at most 256 KB).
const MAX_CHUNK: usize = 8 * 1024 * 1024;
/// A line that never ends is dropped after this many bytes.
const MAX_PENDING: usize = 8 * 1024 * 1024;
/// The window of turns the phone keeps (the same as the host's first read).
const MAX_TURNS: usize = 3000;

/// What one chunk changed, by turn id, in feed order.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct FeedChange {
    /// New turns appended at the end.
    pub added: Vec<String>,
    /// Earlier turns changed by a later record (a tool result, the next block of the same message).
    pub updated: Vec<String>,
    /// Turns dropped from the front because the window is full.
    pub dropped: u32,
    /// Number of turns now.
    pub total: u32,
}

#[derive(Default)]
struct Inner {
    parser: TranscriptParser,
    /// The last line of the data so far when it has no line break yet.
    pending: String,
}

/// Turns of one session, updated by events.
#[derive(uniffi::Object)]
pub struct FeedState {
    inner: Mutex<Inner>,
}

impl Default for FeedState {
    fn default() -> Self {
        Self::new()
    }
}

#[uniffi::export]
impl FeedState {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Adds new data of the session file (any length, cut anywhere). Returns what changed.
    pub fn apply_chunk(&self, chunk: String) -> FeedChange {
        let Ok(mut inner) = self.inner.lock() else {
            return FeedChange::default();
        };
        if chunk.len() > MAX_CHUNK {
            return FeedChange {
                total: inner.parser.turns.len() as u32,
                ..FeedChange::default()
            };
        }
        let before = inner.parser.turns.clone();
        let mut data = std::mem::take(&mut inner.pending);
        data.push_str(&chunk);
        let complete = data.rfind('\n').map_or(0, |i| i + 1);
        for line in data[..complete].lines() {
            inner.parser.feed_line(line);
        }
        let rest = &data[complete..];
        if rest.len() <= MAX_PENDING {
            inner.pending = rest.to_string();
        }
        let mut change = FeedChange::default();
        for (i, t) in inner.parser.turns.iter().enumerate() {
            match before.get(i) {
                None => change.added.push(t.id.clone()),
                Some(old) if old != t => change.updated.push(t.id.clone()),
                Some(_) => {}
            }
        }
        let over = inner.parser.turns.len().saturating_sub(MAX_TURNS);
        if over > 0 {
            inner.parser.turns.drain(..over);
            change.dropped = over as u32;
            let gone: std::collections::HashSet<String> =
                before.iter().take(over).map(|t| t.id.clone()).collect();
            change.updated.retain(|id| !gone.contains(id));
        }
        change.total = inner.parser.turns.len() as u32;
        change
    }

    /// Forgets everything (the file was replaced): the next chunk starts a new feed.
    pub fn reset(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            *inner = Inner::default();
        }
    }

    /// All turns now, in feed order.
    pub fn turns(&self) -> Vec<TranscriptTurn> {
        self.inner
            .lock()
            .map(|i| i.parser.turns.clone())
            .unwrap_or_default()
    }

    /// One turn by id.
    pub fn turn(&self, id: String) -> Option<TranscriptTurn> {
        self.inner
            .lock()
            .ok()?
            .parser
            .turns
            .iter()
            .find(|t| t.id == id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::parse_transcript_jsonl;

    const SESSION: &str = concat!(
        r#"{"type":"mode","mode":"normal"}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"t1"}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"thinking","thinking":"hmm"}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"text","text":"Running Привет"}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"tool_use","id":"c1","name":"Bash","input":{"command":"ls"}}]}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":[{"type":"text","text":"a.txt"}]}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"m2","role":"assistant","content":"Done."}}"#,
        "\n",
        r#"{"type":"system","subtype":"turn_duration","durationMs":58030}"#,
        "\n",
    );

    fn whole() -> Vec<TranscriptTurn> {
        parse_transcript_jsonl(SESSION.to_string()).expect("parse")
    }

    #[test]
    fn replay_in_one_chunk_equals_the_whole_file() {
        let f = FeedState::new();
        f.apply_chunk(SESSION.to_string());
        assert_eq!(f.turns(), whole());
    }

    #[test]
    fn replay_cut_at_every_byte_equals_the_whole_file() {
        for cut in (0..=SESSION.len()).filter(|i| SESSION.is_char_boundary(*i)) {
            let f = FeedState::new();
            f.apply_chunk(SESSION[..cut].to_string());
            f.apply_chunk(SESSION[cut..].to_string());
            assert_eq!(f.turns(), whole(), "cut at {cut}");
        }
    }

    #[test]
    fn replay_line_by_line_and_in_tiny_chunks_equals_the_whole_file() {
        let f = FeedState::new();
        for l in SESSION.split_inclusive('\n') {
            f.apply_chunk(l.to_string());
        }
        assert_eq!(f.turns(), whole());
        let g = FeedState::new();
        let chars: Vec<char> = SESSION.chars().collect();
        for c in chars.chunks(7) {
            g.apply_chunk(c.iter().collect());
        }
        assert_eq!(g.turns(), whole());
    }

    #[test]
    fn a_tool_result_updates_the_earlier_turn_and_a_new_message_is_added() {
        let f = FeedState::new();
        let lines: Vec<&str> = SESSION.split_inclusive('\n').collect();
        let c1 = f.apply_chunk(lines[..5].concat());
        assert_eq!(
            c1.added.len(),
            2,
            "user turn and the assistant message so far: {c1:?}"
        );
        assert!(f.turns()[1].tool_calls[0].result.is_none());
        let c2 = f.apply_chunk(lines[5].to_string());
        assert!(c2.added.is_empty());
        assert_eq!(c2.updated, vec![f.turns()[1].id.clone()]);
        assert_eq!(f.turns()[1].tool_calls[0].result.as_deref(), Some("a.txt"));
        let c3 = f.apply_chunk(lines[6..].concat());
        assert_eq!(c3.added.len(), 2);
        assert!(c3.updated.is_empty());
        assert_eq!(c3.total, 4);
    }

    #[test]
    fn the_next_block_of_one_message_is_an_update_of_the_same_turn() {
        let f = FeedState::new();
        let lines: Vec<&str> = SESSION.split_inclusive('\n').collect();
        f.apply_chunk(lines[..3].concat());
        let c = f.apply_chunk(lines[3].to_string());
        assert!(c.added.is_empty());
        assert_eq!(c.updated.len(), 1);
        assert_eq!(f.turns()[1].content.as_deref(), Some("Running Привет"));
    }

    #[test]
    fn applying_the_same_lines_twice_is_not_idempotent_so_the_host_cursor_must_prevent_it() {
        // Documented behaviour: a repeated chunk adds repeated turns. The host's (inode, offset)
        // cursor is what prevents a repeat; `reset` is the recovery.
        let f = FeedState::new();
        f.apply_chunk(SESSION.to_string());
        let n = f.turns().len();
        f.apply_chunk(SESSION.to_string());
        assert!(f.turns().len() > n);
        f.reset();
        f.apply_chunk(SESSION.to_string());
        assert_eq!(f.turns(), whole());
    }

    #[test]
    fn garbage_halves_and_floods_are_bounded() {
        let f = FeedState::new();
        let c = f.apply_chunk("not json\n{\"type\":\n\u{1b}[31mx\n{".to_string());
        assert_eq!(c.total, 0);
        // A line that never ends does not grow without bound.
        let huge = "a".repeat(MAX_PENDING + 10);
        f.apply_chunk(huge);
        assert!(f.inner.lock().expect("lock").pending.len() <= MAX_PENDING);
        // More turns than the window keeps the newest ones.
        let mut many = String::new();
        for i in 0..(MAX_TURNS + 50) {
            many.push_str(&format!(
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"m{i}\"}}}}\n"
            ));
        }
        let g = FeedState::new();
        let c = g.apply_chunk(many);
        assert_eq!(c.total as usize, MAX_TURNS);
        assert_eq!(c.dropped, 50);
        assert_eq!(
            g.turns().last().and_then(|t| t.content.clone()),
            Some(format!("m{}", MAX_TURNS + 49))
        );
    }

    #[test]
    fn hostile_file_content_is_survived() {
        let f = FeedState::new();
        // Nesting far beyond any stack: the line is skipped, not a crash.
        let deep = format!("{}1{}\n", "[".repeat(200_000), "]".repeat(200_000));
        f.apply_chunk(deep);
        // A huge single record is accepted once and bounded in memory by the chunk cap.
        let big = format!(
            "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"{}\"}}}}\n",
            "a".repeat(2_000_000)
        );
        let c = f.apply_chunk(big);
        assert_eq!(c.added.len(), 1);
        // Escape sequences and bidi overrides in text do not reach the turn.
        let esc = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"a\\u001b[31mred\\u202Eb\"}}\n";
        f.apply_chunk(esc.to_string());
        let t = f.turns();
        let text = t.last().and_then(|t| t.content.clone()).unwrap_or_default();
        assert!(
            !text.contains('\u{1b}') && !text.contains('\u{202E}'),
            "{text:?}"
        );
        // Binary noise between records.
        f.apply_chunk("\u{0}\u{1}\u{2}\n{\"type\":\"user\"\n".to_string());
    }
}
