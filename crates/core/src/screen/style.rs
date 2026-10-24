//! Styles from the screen: the capture keeps the agent's SGR styling (`capture-pane -e`), and the
//! words of a reply get back the Markdown the agent drew as style: bold, italic, inline code.
//!
//! The parser itself works on the plain text. This pass runs afterwards, on reply text only: it
//! reads the styled screen as a stream of words (each word is a few segments with a style) and
//! lays the styles over the words of a turn, in order. A word that is not found is left plain, so
//! a mismatch costs a style, never text. Nothing of the raw escape sequences is ever copied: only
//! the three markers below are produced.

use super::ScreenDialect;
use crate::transcript::{TranscriptTurn, TranscriptTurnType};

const BOLD: u8 = 1;
const ITALIC: u8 = 2;
const CODE: u8 = 4;

/// How far ahead a word is looked for without a second word to confirm it.
const NEAR: usize = 12;
/// How far ahead a pair of words is looked for (after a code block or a folded part).
const FAR: usize = 3000;

#[derive(Debug, Clone)]
pub(super) struct StyledWord {
    text: String,
    /// `(text, style)` pieces; their text joined is `text`.
    segs: Vec<(String, u8)>,
}

/// Foreground colour a dialect draws inline code with.
fn is_code_colour(dialect: ScreenDialect, params: &[u32]) -> bool {
    match dialect {
        // Claude: 256-colour 153. agy: bright blue (94).
        ScreenDialect::Claude => params == [38, 5, 153],
        ScreenDialect::Agy => params == [94],
        ScreenDialect::Unknown => false,
    }
}

#[derive(Default)]
struct State {
    bold: bool,
    italic: bool,
    code: bool,
}

impl State {
    fn style(&self) -> u8 {
        if self.code {
            CODE
        } else {
            u8::from(self.bold) * BOLD + u8::from(self.italic) * ITALIC
        }
    }

    fn apply(&mut self, dialect: ScreenDialect, params: &[u32]) {
        if params.is_empty() {
            *self = State::default();
            return;
        }
        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => *self = State::default(),
                1 => self.bold = true,
                22 => self.bold = false,
                3 => self.italic = true,
                23 => self.italic = false,
                39 => self.code = false,
                38 => {
                    // 38;5;N or 38;2;R;G;B
                    let take = match params.get(i + 1) {
                        Some(5) => 3,
                        Some(2) => 5,
                        _ => 1,
                    };
                    let end = (i + take).min(params.len());
                    self.code = is_code_colour(dialect, &params[i..end]);
                    i = end;
                    continue;
                }
                n @ 30..=37 | n @ 90..=97 => self.code = is_code_colour(dialect, &[n]),
                _ => {}
            }
            i += 1;
        }
    }
}

/// The styled words of a raw capture, in screen order.
pub(super) fn styled_words(raw: &str, dialect: ScreenDialect) -> Vec<StyledWord> {
    let mut out: Vec<StyledWord> = Vec::new();
    let mut state = State::default();
    let mut segs: Vec<(String, u8)> = Vec::new();
    let flush = |segs: &mut Vec<(String, u8)>, out: &mut Vec<StyledWord>| {
        if !segs.is_empty() {
            let text = segs.iter().map(|s| s.0.as_str()).collect::<String>();
            out.push(StyledWord {
                text,
                segs: std::mem::take(segs),
            });
        }
    };
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    let mut body = String::new();
                    for d in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&d) {
                            if d == 'm' {
                                let params: Vec<u32> = body
                                    .split(';')
                                    .map(|p| p.parse::<u32>().unwrap_or(0))
                                    .collect();
                                let params = if body.is_empty() { vec![] } else { params };
                                state.apply(dialect, &params);
                            }
                            break;
                        }
                        body.push(d);
                    }
                }
                Some(']') => {
                    // OSC: up to BEL or ST.
                    chars.next();
                    while let Some(d) = chars.next() {
                        if d == '\u{7}' || (d == '\u{1b}' && chars.peek() == Some(&'\\')) {
                            if d == '\u{1b}' {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            },
            c if c.is_whitespace() => flush(&mut segs, &mut out),
            c if c.is_control() => {}
            c => {
                let style = state.style();
                match segs.last_mut() {
                    Some((t, s)) if *s == style => t.push(c),
                    _ => segs.push((c.to_string(), style)),
                }
            }
        }
    }
    flush(&mut segs, &mut out);
    out
}

fn locate(stream: &[StyledWord], cursor: usize, toks: &[&str]) -> Option<usize> {
    let end = stream.len().min(cursor + NEAR);
    if let Some(i) = (cursor..end).find(|&i| stream[i].text == toks[0]) {
        return Some(i);
    }
    let second = *toks.get(1)?;
    let end = stream.len().min(cursor + FAR);
    (end.min(cursor + NEAR)..end)
        .find(|&i| stream[i].text == toks[0] && stream.get(i + 1).is_some_and(|w| w.text == second))
}

/// Wraps one run of text in the Markdown for `style`.
fn wrap(text: &str, style: u8) -> String {
    if style == 0 || !text.chars().any(char::is_alphanumeric) {
        return text.to_string();
    }
    if style == CODE {
        return if text.contains('`') {
            text.to_string()
        } else {
            format!("`{text}`")
        };
    }
    let mark = match (style & BOLD != 0, style & ITALIC != 0) {
        (true, true) => "***",
        (true, false) => "**",
        _ => "*",
    };
    if text.contains('*') {
        return text.to_string();
    }
    format!("{mark}{text}{mark}")
}

/// Lays the stream's styles over `content`, moving `cursor` forward through the stream.
fn restyle(
    content: &str,
    stream: &[StyledWord],
    cursor: &mut usize,
    dialect: ScreenDialect,
) -> String {
    let mut out = String::with_capacity(content.len() + 16);
    let mut fenced = false;
    let mut prev_line = "";
    for (n, line) in content.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            out.push_str(line);
            prev_line = line;
            continue;
        }
        if fenced {
            out.push_str(line);
            prev_line = line;
            continue;
        }
        // Tokens of the line: (is_space, text). Words get styled pieces, spaces get the style
        // both neighbours share, so `**a** **b**` is one bold run `**a b**`.
        let words: Vec<&str> = line.split_whitespace().collect();
        let mut pieces: Vec<(String, u8)> = Vec::new(); // text, style
        let mut word_idx = 0usize;
        let mut rest = line;
        while !rest.is_empty() {
            let ws_len = rest.len() - rest.trim_start().len();
            if ws_len > 0 {
                pieces.push((rest[..ws_len].to_string(), u8::MAX)); // MAX: a gap, decided below
                rest = &rest[ws_len..];
                continue;
            }
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let word = &rest[..end];
            rest = &rest[end..];
            let hit = locate(
                stream,
                *cursor,
                &words[word_idx..words.len().min(word_idx + 2)],
            );
            word_idx += 1;
            match hit {
                Some(i) => {
                    *cursor = i + 1;
                    let w = &stream[i];
                    for (t, s) in &w.segs {
                        pieces.push((t.clone(), *s));
                    }
                }
                None => pieces.push((word.to_string(), 0)),
            }
        }
        // A gap takes the style its neighbours share (bold/italic carry across words; code only over one space).
        for i in 0..pieces.len() {
            if pieces[i].1 == u8::MAX {
                let before = i.checked_sub(1).map(|j| pieces[j].1).unwrap_or(0);
                let after = pieces.get(i + 1).map_or(0, |p| p.1);
                // Code words side by side form one command (`git push --force`), a lone word stays
                // a word; only a single space joins them.
                let joins_code = before == CODE && pieces[i].0 == " ";
                pieces[i].1 =
                    if before == after && before != u8::MAX && (before != CODE || joins_code) {
                        before
                    } else {
                        0
                    };
            }
        }
        // Claude draws a heading as a bold line of its own; a Markdown heading says it better.
        let before = if n == 0 { "" } else { prev_line };
        if dialect == ScreenDialect::Claude && is_bold_heading(&pieces, line, before) {
            let text: String = pieces.iter().map(|p| p.0.as_str()).collect();
            out.push_str("### ");
            out.push_str(text.trim());
            prev_line = line;
            continue;
        }
        let mut i = 0;
        while i < pieces.len() {
            let style = pieces[i].1;
            let mut j = i;
            let mut text = String::new();
            while j < pieces.len() && pieces[j].1 == style {
                text.push_str(&pieces[j].0);
                j += 1;
            }
            out.push_str(&wrap(&text, style));
            i = j;
        }
        prev_line = line;
    }
    out
}

/// A line that is bold from start to end, short, unpunctuated at the end, and starts a block
/// (first line or after a blank one): not a list item, not a sentence.
fn is_bold_heading(pieces: &[(String, u8)], line: &str, before: &str) -> bool {
    let text: String = pieces.iter().map(|p| p.0.as_str()).collect();
    let text = text.trim();
    !text.is_empty()
        && text.chars().count() <= 80
        && before.trim().is_empty()
        && !line.trim_start().starts_with(['-', '•', '>', '|', '#'])
        && !text.ends_with(['.', ',', ';', ':'])
        && pieces
            .iter()
            .filter(|p| !p.0.trim().is_empty())
            .all(|p| p.1 == BOLD)
}

/// Restyles the reply text of the turns the screen produced.
pub(super) fn restyle_turns(turns: &mut [TranscriptTurn], raw: &str, dialect: ScreenDialect) {
    if !raw.contains('\u{1b}') {
        return;
    }
    let stream = styled_words(raw, dialect);
    if stream.iter().all(|w| w.segs.iter().all(|s| s.1 == 0)) {
        return;
    }
    let mut cursor = 0;
    for t in turns.iter_mut() {
        if t.turn_type != TranscriptTurnType::Assistant || !t.tool_calls.is_empty() {
            continue;
        }
        if let Some(content) = t.content.as_mut() {
            *content = restyle(content, &stream, &mut cursor, dialect);
        }
    }
}

#[cfg(test)]
mod style_tests {
    use super::*;

    fn run(raw: &str, content: &str, d: ScreenDialect) -> String {
        let s = styled_words(raw, d);
        let mut c = 0;
        restyle(content, &s, &mut c, d)
    }

    #[test]
    fn claude_bold_words_become_one_run_and_code_colour_becomes_backticks() {
        let raw = "\u{1b}[38;5;231m⏺\u{1b}[39m \u{1b}[1mSpecs\u{1b}[0m \u{1b}[1m37–43\u{1b}[0m done (\u{1b}[38;5;153m364713f\u{1b}[39m).";
        let got = run(raw, "Specs 37–43 done (364713f).", ScreenDialect::Claude);
        assert_eq!(got, "**Specs 37–43** done (`364713f`).");
    }

    #[test]
    fn agy_italic_bold_and_code() {
        let raw =
            "\u{1b}[3mnote\u{1b}[0m and \u{1b}[1mstrong\u{1b}[0m use \u{1b}[94mcargo\u{1b}[39m now";
        let got = run(raw, "note and strong use cargo now", ScreenDialect::Agy);
        assert_eq!(got, "*note* and **strong** use `cargo` now");
    }

    #[test]
    fn code_words_side_by_side_become_one_command() {
        let c = "\u{1b}[38;5;153m";
        let e = "\u{1b}[39m";
        let raw = format!("run {c}git{e} {c}push{e} {c}--force{e}, then {c}ls{e} ok");
        let got = run(
            &raw,
            "run git push --force, then ls ok",
            ScreenDialect::Claude,
        );
        assert_eq!(got, "run `git push --force`, then `ls` ok");
    }

    #[test]
    fn unmatched_words_stay_plain_and_text_is_never_lost() {
        let raw = "\u{1b}[1mhello\u{1b}[0m world";
        let got = run(raw, "something else entirely", ScreenDialect::Claude);
        assert_eq!(got, "something else entirely");
    }

    #[test]
    fn code_blocks_and_other_colours_are_left_alone() {
        let raw = "\u{1b}[38;5;246mgrey\u{1b}[39m \u{1b}[1mkeep\u{1b}[0m";
        assert_eq!(
            run(raw, "grey keep", ScreenDialect::Claude),
            "grey **keep**"
        );
        let raw = "\u{1b}[1mx\u{1b}[0m";
        assert_eq!(
            run(raw, "```\nx\n```", ScreenDialect::Claude),
            "```\nx\n```"
        );
    }

    #[test]
    fn reflowed_lines_and_table_pipes_still_match_in_order() {
        let raw = "\u{1b}[1malpha\u{1b}[0m beta\ngamma \u{1b}[1mdelta\u{1b}[0m";
        let got = run(raw, "alpha beta gamma delta", ScreenDialect::Claude);
        assert_eq!(got, "**alpha** beta gamma **delta**");
    }

    #[test]
    fn a_bold_line_of_its_own_becomes_a_heading_in_claude_only() {
        let raw = "\u{1b}[1mNext steps\u{1b}[0m\n1. one\n\n\u{1b}[1mSentence that is bold.\u{1b}[0m\n\u{1b}[1mHead\u{1b}[0m";
        let content = "Next steps\n1. one\n\nSentence that is bold.\nHead";
        let got = run(raw, content, ScreenDialect::Claude);
        assert_eq!(
            got,
            "### Next steps\n1. one\n\n**Sentence that is bold.**\nHead"
                .replace("\nHead", "\n**Head**")
        );
        // agy writes its own `###`; a bold line stays bold there
        assert!(run(raw, content, ScreenDialect::Agy).starts_with("**Next steps**"));
    }

    #[test]
    fn hostile_sequences_are_not_copied() {
        let raw = "\u{1b}]0;title\u{7}\u{1b}[2Jhi \u{1b}[1mthere\u{1b}[0m\u{1b}[?25l";
        let got = run(raw, "hi there", ScreenDialect::Claude);
        assert_eq!(got, "hi **there**");
        assert!(!got.contains('\u{1b}'));
    }
}
