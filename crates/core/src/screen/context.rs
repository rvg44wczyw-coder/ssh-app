//! Claude Code's `/context` output read from the pane: how many tokens each part of the context
//! costs (system prompt, tools, skills, memory files, messages) and what each skill costs.
//! Pure text in, typed numbers out; the pane is untrusted display data, so every number is
//! parsed strictly and the lists are capped.

const MAX_SKILLS: usize = 300;
const MAX_CATEGORIES: usize = 20;
const MAX_TOKENS: u64 = 100_000_000;

/// One line of "Estimated usage by category".
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ContextCategory {
    pub name: String,
    pub tokens: u64,
    pub percent: f32,
}

/// One skill of the "Skills" tree with the group it comes from ("User", "Plugin (design)").
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContextSkill {
    pub group: String,
    pub name: String,
    /// `< 20 tokens` is reported as 10.
    pub tokens: u64,
}

/// A `/context` report.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ContextUsage {
    pub model: String,
    pub used_tokens: u64,
    pub window_tokens: u64,
    pub categories: Vec<ContextCategory>,
    pub skills: Vec<ContextSkill>,
}

/// Reads the last `/context` report on the pane. `None`: there is none on the screen.
#[uniffi::export]
pub fn parse_context_usage(text: String) -> Option<ContextUsage> {
    let text = crate::transcript::strip_ansi_codes(text);
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .rposition(|l| l.trim_end().ends_with("Context Usage"))?;
    let mut usage = ContextUsage {
        model: String::new(),
        used_tokens: 0,
        window_tokens: 0,
        categories: Vec::new(),
        skills: Vec::new(),
    };
    let mut in_skills = false;
    let mut group = String::new();
    for raw in &lines[start + 1..] {
        let line = raw.trim_end();
        let body = line.trim_start_matches(|c: char| c.is_whitespace() || "⎿⛁⛀⛶⛝".contains(c));
        if body.starts_with("Skills") && body.contains('/') && !body.contains("tokens") {
            in_skills = true;
            group.clear();
            continue;
        }
        if in_skills {
            if let Some(rest) = tree_item(body) {
                if let Some((name, tokens)) = split_tokens(rest) {
                    if usage.skills.len() < MAX_SKILLS && !group.is_empty() {
                        usage.skills.push(ContextSkill {
                            group: group.clone(),
                            name,
                            tokens,
                        });
                    }
                }
            } else if line.starts_with("❯") || line.starts_with('─') {
                break;
            } else if !body.is_empty() && line.starts_with("     ") && !body.contains(" · ") {
                group = body.chars().take(60).collect();
            } else if !body.is_empty() && !line.starts_with("     ") {
                break;
            }
            continue;
        }
        if let Some((used, window)) = parse_window(body) {
            usage.used_tokens = used;
            usage.window_tokens = window;
        } else if usage.window_tokens == 0
            && usage.model.is_empty()
            && !body.is_empty()
            && !body.contains([':', '/', '('])
        {
            usage.model = body.chars().take(60).collect();
        } else if let Some(cat) = parse_category(body) {
            if usage.categories.len() < MAX_CATEGORIES {
                usage.categories.push(cat);
            }
        }
    }
    (usage.window_tokens > 0 || !usage.categories.is_empty()).then_some(usage)
}

fn tree_item(body: &str) -> Option<&str> {
    body.strip_prefix('├')
        .or_else(|| body.strip_prefix('└'))
        .map(str::trim)
}

/// `name: ~220 tokens`, `name: < 20 tokens`, `name: 361 tokens`.
fn split_tokens(rest: &str) -> Option<(String, u64)> {
    let (name, value) = rest.rsplit_once(':')?;
    let value = value.trim().trim_start_matches('~').trim();
    let tokens = if let Some(v) = value.strip_prefix('<') {
        v.trim()
            .strip_suffix("tokens")?
            .trim()
            .parse::<u64>()
            .ok()
            .map(|_| 10)?
    } else {
        parse_amount(value.strip_suffix("tokens")?.trim())?
    };
    let name: String = name.trim().chars().take(100).collect();
    (!name.is_empty()).then_some((name, tokens))
}

/// `3k`, `13.2k`, `772`, `1.5m`.
fn parse_amount(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, mult) = match s.chars().last()? {
        'k' | 'K' => (&s[..s.len() - 1], 1_000.0),
        'm' | 'M' => (&s[..s.len() - 1], 1_000_000.0),
        _ => (s, 1.0),
    };
    if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let v: f64 = num.parse().ok()?;
    let v = (v * mult).round();
    (v >= 0.0 && v <= MAX_TOKENS as f64).then_some(v as u64)
}

/// `30k/400k tokens (7%)`.
fn parse_window(body: &str) -> Option<(u64, u64)> {
    let head = body.split(" tokens").next()?;
    let (used, window) = head.split_once('/')?;
    Some((parse_amount(used)?, parse_amount(window)?))
}

/// `System prompt: 3k tokens (0.7%)`, `Free space: 337k (84.3%)`.
fn parse_category(body: &str) -> Option<ContextCategory> {
    let (name, rest) = body.split_once(": ")?;
    let (amount, pct) = rest.split_once(" (")?;
    let tokens = parse_amount(
        amount
            .trim()
            .strip_suffix(" tokens")
            .unwrap_or(amount.trim()),
    )?;
    let percent: f32 = pct.strip_suffix("%)")?.trim().parse().ok()?;
    let name: String = name.trim().chars().take(60).collect();
    (!name.is_empty() && (0.0..=1000.0).contains(&percent)).then_some(ContextCategory {
        name,
        tokens,
        percent,
    })
}
