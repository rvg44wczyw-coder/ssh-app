use super::*;

/// Subagents (Claude's `Task` / `Agent` tool, Antigravity's `invoke_subagent`) the session has started. Claude hands searching to
/// them often, so the feed shows how many are working now and how many ran in total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, uniffi::Record)]
pub struct SubagentStats {
    pub running: u32,
    pub total: u32,
}

pub(super) fn is_subagent_call(tool_name: &str) -> bool {
    matches!(tool_name, "Task" | "Agent" | "invoke_subagent")
}

/// Counts subagent calls in the turns; a call without a result yet is running.
pub fn subagent_stats_of(turns: &[TranscriptTurn]) -> SubagentStats {
    let mut stats = SubagentStats::default();
    for call in turns.iter().flat_map(|t| &t.tool_calls) {
        if is_subagent_call(&call.tool_name) {
            stats.total += 1;
            if call.status == TranscriptStatus::Running {
                stats.running += 1;
            }
        }
    }
    stats
}

/// UniFFI form of [`subagent_stats_of`].
#[uniffi::export]
pub fn subagent_stats(turns: Vec<TranscriptTurn>) -> SubagentStats {
    subagent_stats_of(&turns)
}
