fn task_call(id: &str, status: TranscriptStatus) -> TranscriptToolCall {
    TranscriptToolCall {
        call_id: id.into(),
        tool_name: "Task".into(),
        arguments_summary: String::new(),
        full_arguments_json: None,
        result: None,
        is_error: false,
        status,
    }
}

#[test]
fn subagent_calls_are_counted_running_and_total() {
    let turn = |calls| TranscriptTurn {
        id: "t".into(),
        step_index: 0,
        turn_type: TranscriptTurnType::Assistant,
        timestamp: None,
        content: None,
        thinking: None,
        tool_calls: calls,
        status: TranscriptStatus::Done,
        label: None,
    };
    let mut bash = task_call("b", TranscriptStatus::Running);
    bash.tool_name = "Bash".into();
    let turns = vec![
        turn(vec![task_call("1", TranscriptStatus::Done), bash]),
        turn(vec![task_call("2", TranscriptStatus::Running)]),
    ];
    assert_eq!(
        subagent_stats_of(&turns),
        SubagentStats {
            running: 1,
            total: 2
        }
    );
    assert_eq!(subagent_stats_of(&[]), SubagentStats::default());
}

#[test]
fn a_subagent_call_is_summarised_by_kind_and_description_not_by_its_prompt() {
    let input = serde_json::json!({
        "description": "Find the auth code",
        "subagent_type": "Explore",
        "prompt": "Search the whole repository for ... a very long brief"
    });
    assert_eq!(summarize_arguments(&input), "Explore: Find the auth code");
    let no_kind = serde_json::json!({"description": "Look around", "prompt": "x"});
    assert_eq!(summarize_arguments(&no_kind), "Look around");
}

use super::*;

#[test]
fn test_strip_ansi_codes() {
    let colored = "\x1B[32mSuccess!\x1B[0m\x1B[1m Bold\x1B[0m";
    let cleaned = strip_ansi_codes(colored.to_string());
    assert_eq!(cleaned, "Success! Bold");
}

#[test]
fn test_parse_antigravity_jsonl() {
    let jsonl = r#"
{"step_index": 1, "source": "USER_EXPLICIT", "type": "USER_INPUT", "status": "DONE", "created_at": "2026-10-08T15:00:00Z", "content": "build the project"}
{"step_index": 2, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "created_at": "2026-10-08T15:00:01Z", "thinking": "I need to run cargo build", "content": "Running build now...", "tool_calls": [{"name": "run_command", "arguments": {"CommandLine": "cargo build"}}]}
{"step_index": 3, "source": "TOOL", "type": "TOOL_RESPONSE", "status": "DONE", "content": "\u001b[32mFinished\u001b[0m dev [unoptimized + debuginfo]"}
"#;

    let turns = parse_transcript_jsonl(jsonl.to_string()).expect("Should parse successfully");
    assert_eq!(turns.len(), 2);

    // Turn 1: User
    assert_eq!(turns[0].step_index, 1);
    assert_eq!(turns[0].turn_type, TranscriptTurnType::User);
    assert_eq!(turns[0].content.as_deref(), Some("build the project"));

    // Turn 2: Assistant with thinking and tool call with attached result
    assert_eq!(turns[1].step_index, 2);
    assert_eq!(turns[1].turn_type, TranscriptTurnType::Assistant);
    assert_eq!(
        turns[1].thinking.as_deref(),
        Some("I need to run cargo build")
    );
    assert_eq!(turns[1].content.as_deref(), Some("Running build now..."));
    assert_eq!(turns[1].tool_calls.len(), 1);

    let tc = &turns[1].tool_calls[0];
    assert_eq!(tc.tool_name, "run_command");
    assert_eq!(tc.arguments_summary, "cargo build");
    assert_eq!(
        tc.result.as_deref(),
        Some("Finished dev [unoptimized + debuginfo]")
    );
    assert_eq!(tc.status, TranscriptStatus::Done);
}

#[test]
fn test_parse_claude_code_jsonl() {
    let jsonl = r#"
{"role": "user", "content": "What is the status?"}
{"role": "assistant", "content": [{"type": "thinking", "thinking": "Let me check git status"}, {"type": "text", "text": "Checking repository..."}, {"type": "tool_use", "id": "call_git", "name": "Bash", "input": {"command": "git status -s"}}]}
{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "call_git", "content": "M crates/core/src/lib.rs", "is_error": false}]}
"#;

    let turns = parse_transcript_jsonl(jsonl.to_string()).expect("Should parse successfully");
    assert_eq!(turns.len(), 3);

    assert_eq!(turns[0].turn_type, TranscriptTurnType::User);
    assert_eq!(turns[0].content.as_deref(), Some("What is the status?"));

    assert_eq!(turns[1].turn_type, TranscriptTurnType::Assistant);
    assert_eq!(
        turns[1].thinking.as_deref(),
        Some("Let me check git status")
    );
    assert_eq!(turns[1].content.as_deref(), Some("Checking repository..."));
    assert_eq!(turns[1].tool_calls.len(), 1);

    let tc = &turns[1].tool_calls[0];
    assert_eq!(tc.call_id, "call_git");
    assert_eq!(tc.arguments_summary, "git status -s");
    assert_eq!(tc.result.as_deref(), Some("M crates/core/src/lib.rs"));
    assert_eq!(tc.status, TranscriptStatus::Done);
}

#[test]
fn test_parse_claude_code_session_file() {
    let jsonl = r#"
{"type":"mode","mode":"normal"}
{"type":"api-request","id":"x"}
{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"t1"}
{"type":"user","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>x</local-command-caveat>"}}
{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"thinking","thinking":"hmm"}]}}
{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"text","text":"Running"}]}}
{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"tool_use","id":"c1","name":"Bash","input":{"command":"ls"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":[{"type":"text","text":"a.txt"}]}]}}
{"type":"assistant","isSidechain":true,"message":{"id":"s","role":"assistant","content":"sub"}}
{"type":"system","subtype":"turn_duration","durationMs":58030}
{"type":"attachment","attachment":{"type":"edited_text_file","filename":"/a/b"}}
{"type":"attachment","attachment":{"type":"total_tokens_reminder","text":"<total_tokens>14964699 tokens left</total_tokens>"}}
{"type":"attachment","attachment":{"type":"credential_org","organizationUuid":"x"}}
"#;
    let turns = parse_transcript_jsonl(jsonl.to_string()).unwrap();
    assert_eq!(turns.len(), 6);
    assert_eq!(turns[0].turn_type, TranscriptTurnType::User);
    assert_eq!(turns[0].content.as_deref(), Some("hi"));
    assert_eq!(turns[1].label.as_deref(), Some("Service message"));
    assert_eq!(turns[2].turn_type, TranscriptTurnType::Assistant);
    assert_eq!(turns[2].thinking.as_deref(), Some("hmm"));
    assert_eq!(turns[2].content.as_deref(), Some("Running"));
    assert_eq!(turns[2].tool_calls.len(), 1);
    assert_eq!(turns[2].tool_calls[0].result.as_deref(), Some("a.txt"));
    assert_eq!(turns[2].tool_calls[0].status, TranscriptStatus::Done);
    assert_eq!(turns[3].label.as_deref(), Some("Worked for 58s"));
    assert_eq!(turns[4].label.as_deref(), Some("File changed"));
    assert_eq!(turns[4].content.as_deref(), Some("/a/b"));
    assert_eq!(turns[5].label.as_deref(), Some("Tokens left"));
    assert_eq!(turns[5].content.as_deref(), Some("14,964,699 tokens left"));
}

#[test]
fn test_parse_antigravity_real_shapes() {
    let jsonl = r#"
{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"<USER_REQUEST>\nbuild it\n</USER_REQUEST>"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"run_command","args":{"CommandLine":"\"ls -la\""}}]}
{"step_index":2,"source":"MODEL","type":"GENERIC","status":"DONE","content":"Created At: a\nCompleted At: b\n\nThe command exited with code 0.\nOutput:\nx"}
{"step_index":3,"source":"SYSTEM","type":"CHECKPOINT","content":"Resuming"}
{"step_index":4,"source":"SYSTEM","type":"ERROR_MESSAGE","error":"API error 503"}
"#;
    let turns = parse_transcript_jsonl(jsonl.to_string()).unwrap();
    assert_eq!(turns.len(), 4);
    assert_eq!(turns[0].content.as_deref(), Some("build it"));
    assert_eq!(turns[1].tool_calls[0].arguments_summary, "ls -la");
    assert!(turns[1].tool_calls[0]
        .result
        .as_deref()
        .unwrap()
        .starts_with("The command exited"));
    assert_eq!(
        turns[2].label.as_deref(),
        Some("Checkpoint (context compacted)")
    );
    assert_eq!(turns[3].content.as_deref(), Some("API error 503"));
}

#[test]
fn test_payload_too_large() {
    let large_payload = "a".repeat(MAX_PAYLOAD_SIZE + 10);
    let result = parse_transcript_jsonl(large_payload);
    assert!(matches!(
        result,
        Err(SshCoreError::TranscriptTooLarge { .. })
    ));
}

#[test]
fn test_validate_transcript_path() {
    assert!(validate_transcript_path("/Users/test/transcript.jsonl").is_ok());
    assert!(validate_transcript_path("~/.gemini/transcript.jsonl").is_ok());
    assert!(validate_transcript_path("/etc/shadow").is_err());
    assert!(validate_transcript_path("~/.ssh/id_rsa").is_err());
    assert!(validate_transcript_path("/Users/test/.ssh/id_ed25519").is_err());
    assert!(validate_transcript_path("/Users/test/../../etc/passwd").is_err());
    assert!(validate_transcript_path("-f /etc/passwd").is_err());
    assert!(validate_transcript_path("path/with/'quotes").is_err());
    assert!(validate_transcript_path("").is_err());
}

#[test]
fn test_malformed_lines_are_skipped() {
    let jsonl = "not a json line\n{\"step_index\": 1, \"source\": \"USER_EXPLICIT\", \"type\": \"USER_INPUT\", \"status\": \"DONE\", \"content\": \"hello\"}\n{broken json";
    let turns = parse_transcript_jsonl(jsonl.to_string()).expect("Should parse valid lines");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].content.as_deref(), Some("hello"));
}

#[test]
fn a_message_typed_while_the_agent_works_is_a_user_turn() {
    let raw = concat!(
        r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":"собери билд","origin":{"kind":"human"},"humanTurn":true}}"#,
        "\n",
        r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":"task done","origin":{"kind":"task-notification"}}}"#,
        "\n"
    );
    let turns = parse_transcript_jsonl(raw.to_string()).unwrap();
    assert_eq!(turns[0].turn_type, TranscriptTurnType::User);
    assert_eq!(turns[0].content.as_deref(), Some("собери билд"));
    assert_eq!(turns[1].turn_type, TranscriptTurnType::System);
}
