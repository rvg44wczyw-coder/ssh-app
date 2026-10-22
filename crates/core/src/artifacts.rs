//! Diagrams and reports in an agent's reply (feature 41). Input is the text of an assistant turn
//! from Claude's session file; the screen feed has no code fences, so it gives no artifacts.
//! Only closed fences count: a reply still being streamed never yields half a diagram.

use sha2::{Digest, Sha256};

/// A diagram above either limit is not drawn until the user confirms (a huge diagram freezes the
/// web view).
pub const MAX_RENDER_LINES: usize = 500;
pub const MAX_RENDER_BYTES: usize = 50 * 1024;
/// Artifacts taken from one text; the rest are ignored.
const MAX_ARTIFACTS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ArtifactKind {
    Mermaid,
    Markdown,
    Html,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ExtractedArtifact {
    /// First 16 hex digits of the SHA-256 of the kind and the content: the same block gives the
    /// same id wherever it is found.
    pub id: String,
    pub title: String,
    pub kind: ArtifactKind,
    /// `flowchart`, `sequenceDiagram`, ... for Mermaid; empty otherwise.
    pub chart_type: String,
    pub content: String,
    pub line_count: u32,
    /// Over the render limits: ask before drawing.
    pub needs_confirmation: bool,
}

/// An opening fence: up to 3 spaces of indent, 3+ backticks or tildes, then the info string.
fn open_fence(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let c = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = rest.chars().take_while(|x| *x == c).count();
    if n < 3 {
        return None;
    }
    let info = rest[n..].trim();
    // A backtick fence's info string cannot contain a backtick (that is inline code).
    if c == '`' && info.contains('`') {
        return None;
    }
    Some((c, n, info))
}

fn closes(line: &str, c: char, n: usize) -> bool {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return false;
    }
    let rest = line[indent..].trim_end();
    rest.len() >= n && rest.chars().all(|x| x == c)
}

fn chart_type(source: &str) -> String {
    source
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("%%") && !l.starts_with("---"))
        .and_then(|l| l.split_whitespace().next())
        .map(|w| w.chars().take(40).collect())
        .unwrap_or_default()
}

fn title_for(kind: &ArtifactKind, chart: &str, content: &str) -> String {
    match kind {
        ArtifactKind::Mermaid if !chart.is_empty() => format!("Diagram: {chart}"),
        ArtifactKind::Mermaid => "Diagram".into(),
        ArtifactKind::Markdown => {
            let heading = content
                .lines()
                .map(str::trim)
                .find(|l| l.starts_with('#'))
                .map(|l| l.trim_start_matches('#').trim());
            match heading {
                Some(h) if !h.is_empty() => h.chars().take(80).collect(),
                _ => "Report".into(),
            }
        }
        ArtifactKind::Html => "HTML snippet".into(),
    }
}

#[uniffi::export]
pub fn extract_artifacts(text: String) -> Vec<ExtractedArtifact> {
    let mut out: Vec<ExtractedArtifact> = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some((c, n, info)) = open_fence(line) else {
            continue;
        };
        let lang = info
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let mut body: Vec<&str> = Vec::new();
        let mut closed = false;
        for l in lines.by_ref() {
            if closes(l, c, n) {
                closed = true;
                break;
            }
            body.push(l);
        }
        if !closed {
            break; // the rest of the text is inside an unfinished fence
        }
        let kind = match lang.as_str() {
            "mermaid" => ArtifactKind::Mermaid,
            "markdown" | "md" => ArtifactKind::Markdown,
            "html" => ArtifactKind::Html,
            _ => continue,
        };
        let content = body.join("\n");
        if content.trim().is_empty() {
            continue;
        }
        let chart = if kind == ArtifactKind::Mermaid {
            chart_type(&content)
        } else {
            String::new()
        };
        let mut h = Sha256::new();
        h.update(format!("{kind:?}\n{content}"));
        let id = hex::encode(h.finalize())[..16].to_string();
        if out.iter().any(|a| a.id == id) {
            continue;
        }
        let line_count = content.lines().count();
        out.push(ExtractedArtifact {
            id,
            title: title_for(&kind, &chart, &content),
            needs_confirmation: line_count > MAX_RENDER_LINES || content.len() > MAX_RENDER_BYTES,
            line_count: line_count as u32,
            kind,
            chart_type: chart,
            content,
        });
        if out.len() >= MAX_ARTIFACTS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mermaid_blocks_are_found_with_their_chart_type() {
        let t = "Here:\n```mermaid\nflowchart TD\n  A-->B\n```\ntext\n```mermaid\n%% c\nsequenceDiagram\n A->>B: hi\n```\n```mermaid\nstateDiagram-v2\n [*] --> S\n```";
        let a = extract_artifacts(t.into());
        let types: Vec<_> = a.iter().map(|x| x.chart_type.as_str()).collect();
        assert_eq!(types, ["flowchart", "sequenceDiagram", "stateDiagram-v2"]);
        assert!(a.iter().all(|x| x.kind == ArtifactKind::Mermaid));
    }

    #[test]
    fn an_unclosed_fence_gives_nothing() {
        let t = "```mermaid\nflowchart TD\n A-->B\n";
        assert!(extract_artifacts(t.into()).is_empty());
        let t = "```mermaid\nflowchart TD\n A-->B\n```\n```mermaid\nflowchart LR\n";
        assert_eq!(extract_artifacts(t.into()).len(), 1);
    }

    #[test]
    fn text_without_fences_gives_no_artifacts() {
        assert!(extract_artifacts("flowchart TD\n A-->B\n# Title\n".into()).is_empty());
        assert!(extract_artifacts("```rust\nfn main(){}\n```".into()).is_empty());
    }

    #[test]
    fn a_longer_fence_can_hold_a_shorter_one() {
        let t = "````markdown\n# Report\n```mermaid\nflowchart TD\n```\n````";
        let a = extract_artifacts(t.into());
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].kind, ArtifactKind::Markdown);
        assert_eq!(a[0].title, "Report");
        assert!(a[0].content.contains("```mermaid"));
    }

    #[test]
    fn the_same_block_has_the_same_id_and_duplicates_collapse() {
        let t = "```mermaid\nflowchart TD\n A-->B\n```\n```mermaid\nflowchart TD\n A-->B\n```";
        let a = extract_artifacts(t.into());
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].id, extract_artifacts(t.into())[0].id);
        assert_eq!(a[0].id.len(), 16);
    }

    #[test]
    fn a_huge_diagram_waits_for_confirmation() {
        let body = "A-->B\n".repeat(600);
        let t = format!("```mermaid\nflowchart TD\n{body}```");
        assert!(extract_artifacts(t)[0].needs_confirmation);
        let small = extract_artifacts("```mermaid\nflowchart TD\n A-->B\n```".into());
        assert!(!small[0].needs_confirmation);
    }

    #[test]
    fn html_is_found_and_indented_code_is_not_a_fence() {
        let a = extract_artifacts("```html\n<b>x</b>\n```".into());
        assert_eq!(a[0].kind, ArtifactKind::Html);
        assert!(extract_artifacts("    ```mermaid\n    flowchart TD\n    ```".into()).is_empty());
    }

    #[test]
    fn at_most_twenty_artifacts_are_taken() {
        let t: String = (0..30)
            .map(|i| format!("```mermaid\nflowchart TD\n A{i}-->B\n```\n"))
            .collect();
        assert_eq!(extract_artifacts(t).len(), 20);
    }
}
