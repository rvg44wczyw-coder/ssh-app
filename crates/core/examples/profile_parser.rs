//! Timing of the screen parser on AI-written output of growing size.
//! Run: `cargo run --release -p ssh-core --example profile_parser`
use ssh_core::screen::{merge_thinking, parse_screen_feed, transcript_matches_screen};
use ssh_core::transcript::{
    strip_ansi_codes, TranscriptStatus, TranscriptToolCall, TranscriptTurn, TranscriptTurnType,
};
use std::time::Instant;

fn claude_turn(i: usize, styled: bool) -> String {
    let w = |s: &str| {
        if styled {
            format!("\x1b[38;5;231m{s}\x1b[39m")
        } else {
            s.to_string()
        }
    };
    let mut o = String::new();
    o += &format!(
        "❯ {}\n\n",
        w(&format!(
            "Fix the bug number {i} in the parser and explain what you changed"
        ))
    );
    o += &format!("⏺ {}\n", w("I looked at the parser and found the cause. The loop never reset its state, so a second"));
    o += &format!(
        "  {}\n\n",
        w("pass reused the first pass's cursor. Changes:")
    );
    o += &format!(
        "  - {}\n  - {}\n  - {}\n\n",
        w("reset the cursor at the top of `run()`"),
        w("add a regression test"),
        w("update the docs")
    );
    o += &format!(
        "⏺ Bash(cargo test -p ssh-core --lib screen)\n  ⎿  {}\n",
        w("running 428 tests")
    );
    for k in 0..12 {
        o += &format!(
            "     {}\n",
            w(&format!("test screen::tests::case_{i}_{k} ... ok"))
        );
    }
    o += "     … +40 lines (ctrl+o to expand)\n\n";
    o += &format!("⏺ {}\n\n", w("All tests pass. Summary table:"));
    o += "  ┌──────────┬────────┬──────────┐\n  │ Step     │ Before │ After    │\n  ├──────────┼────────┼──────────┤\n  │ parse    │ 12 ms  │ 3 ms     │\n  │ merge    │ 8 ms   │ 2 ms     │\n  └──────────┴────────┴──────────┘\n\n";
    o += "  ```rust\n  fn run(&mut self) {\n      self.cursor = 0;\n      for t in &self.items { self.step(t); }\n  }\n  ```\n\n";
    o += "✻ Worked for 14s\n\n";
    o
}

fn screen(turns: usize, styled: bool) -> String {
    let mut s = String::new();
    for i in 0..turns {
        s += &claude_turn(i, styled);
    }
    s += &"─".repeat(100);
    s += "\n❯ \n";
    s += &"─".repeat(100);
    s += "\n  Sonnet 5.5 · 5h 14%\n  ⏵⏵ auto mode on (shift+tab to cycle)\n";
    s
}

fn file_turns(n: usize) -> Vec<TranscriptTurn> {
    let mk = |i: usize,
              ty: TranscriptTurnType,
              text: &str,
              thinking: Option<&str>,
              calls: Vec<TranscriptToolCall>| TranscriptTurn {
        id: format!("t{i}"),
        step_index: i as u64,
        turn_type: ty,
        timestamp: None,
        content: Some(text.to_string()),
        thinking: thinking.map(String::from),
        tool_calls: calls,
        status: TranscriptStatus::Done,
        label: None,
    };
    let mut v = Vec::new();
    for i in 0..n {
        v.push(mk(
            2 * i,
            TranscriptTurnType::User,
            &format!("Fix the bug number {i} in the parser and explain what you changed"),
            None,
            vec![],
        ));
        v.push(mk(
            2 * i + 1,
            TranscriptTurnType::Assistant,
            "I looked at the parser and found the cause.",
            Some("Let me think about the loop and its state carefully, step by step."),
            vec![],
        ));
    }
    v
}

fn time<T>(label: &str, runs: u32, mut f: impl FnMut() -> T) {
    let _ = f();
    let t = Instant::now();
    for _ in 0..runs {
        std::hint::black_box(f());
    }
    let per = t.elapsed() / runs;
    println!("  {label:<34} {:>10.3} ms", per.as_secs_f64() * 1000.0);
}

fn main() {
    if let Some(path) = std::env::args().nth(1) {
        jsonl(&path);
        return;
    }
    for turns in [5usize, 20, 100, 400, 1500] {
        let plain = screen(turns, false);
        let styled = screen(turns, true);
        let runs = if turns >= 400 { 3 } else { 20 };
        println!(
            "\n{turns} turns: {} lines, {:.0} KB plain, {:.0} KB styled",
            plain.lines().count(),
            plain.len() as f64 / 1024.0,
            styled.len() as f64 / 1024.0
        );
        time("strip_ansi_codes (styled)", runs, || {
            strip_ansi_codes(styled.clone())
        });
        time("parse_screen_feed plain", runs, || {
            parse_screen_feed(plain.clone())
        });
        time("parse_screen_feed styled (-e)", runs, || {
            parse_screen_feed(styled.clone())
        });
        let feed = parse_screen_feed(styled.clone());
        let file = file_turns(turns);
        time("transcript_matches_screen", runs, || {
            transcript_matches_screen(file.clone(), styled.clone())
        });
        time("merge_thinking", runs, || {
            merge_thinking(feed.turns.clone(), file.clone())
        });
        println!("  turns parsed: {}", feed.turns.len());
    }
}

/// `profile_parser <file.jsonl>`: times the session-file parser on a real tail.
#[allow(dead_code)]
pub fn jsonl(path: &str) {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    let t = Instant::now();
    let turns = ssh_core::transcript::parse_transcript_jsonl(raw.clone()).unwrap_or_default();
    println!(
        "\nsession file tail: {:.2} MB -> {} turns in {:.1} ms",
        raw.len() as f64 / 1048576.0,
        turns.len(),
        t.elapsed().as_secs_f64() * 1000.0
    );
    let feed = ssh_core::feed::FeedState::new();
    let t = Instant::now();
    let mut worst = 0.0f64;
    let mut at = 0;
    while at < raw.len() {
        let mut end = (at + 32 * 1024).min(raw.len());
        while !raw.is_char_boundary(end) {
            end += 1;
        }
        let t1 = Instant::now();
        feed.apply_chunk(raw[at..end].to_string());
        worst = worst.max(t1.elapsed().as_secs_f64() * 1000.0);
        at = end;
    }
    println!(
        "  replay in 32 KB chunks: {:.1} ms total, worst chunk {:.2} ms, equals whole file: {}",
        t.elapsed().as_secs_f64() * 1000.0,
        worst,
        feed.turns() == turns
    );
    let t = Instant::now();
    let same = turns == turns.clone();
    println!(
        "  turns == turns.clone() (the Swift-side diff analogue): {same} in {:.2} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
}
