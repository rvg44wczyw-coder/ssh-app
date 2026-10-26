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

#[test]
fn permission_mode_records_become_one_hidden_turn_per_change() {
    let mut turns = Vec::new();
    let (mut step, mut last) = (0u64, None);
    for mode in ["plan", "plan", "acceptEdits", "bad mode!", ""] {
        let rec = serde_json::json!({"type": "permission-mode", "permissionMode": mode});
        claude::parse_claude_record(&rec, &mut turns, &mut step, &mut last);
    }
    let modes: Vec<_> = turns.iter().map(|t| t.content.as_deref()).collect();
    assert_eq!(modes, vec![Some("plan"), Some("acceptEdits")]);
    assert!(turns
        .iter()
        .all(|t| t.label.as_deref() == Some("permission-mode")));
}

#[test]
fn an_agy_result_that_names_its_call_goes_to_that_call() {
    let jsonl = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"go"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","content":null,"thinking":"t","tool_calls":[{"id":"a","name":"run_command","args":{"CommandLine":"ls"}},{"id":"b","name":"run_command","args":{"CommandLine":"pwd"}}]}
{"step_index":2,"source":"TOOL","type":"TOOL_RESPONSE","content":"result b","tool_call_id":"b","status":"DONE"}
{"step_index":3,"source":"TOOL","type":"TOOL_RESPONSE","content":"result a","tool_call_id":"a","status":"ERROR"}
"#;
    let turns = parse_transcript_jsonl(jsonl.to_string()).unwrap();
    let calls = &turns[1].tool_calls;
    assert_eq!(calls[0].result.as_deref(), Some("result a"));
    assert!(calls[0].is_error);
    assert_eq!(calls[1].result.as_deref(), Some("result b"));
    assert!(!calls[1].is_error);
}

mod codex_rollout {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/golden/transcript/codex_rollout.jsonl");

    #[test]
    fn the_conversation_is_read_and_the_model_side_is_not() {
        let turns = parse_transcript_jsonl(FIXTURE.to_string()).unwrap();
        let kinds: Vec<_> = turns.iter().map(|t| t.turn_type).collect();
        use TranscriptTurnType::*;
        assert_eq!(
            kinds,
            [User, Assistant, Assistant, Assistant, Assistant, System, Assistant, System]
        );
        let all = format!("{turns:?}");
        for hidden in [
            "You are Codex",
            "AGENTS.md",
            "environment dump",
            "tools.exec_command",
        ] {
            assert!(!all.contains(hidden), "{hidden}");
        }
        // A command is one turn with its result; failure comes from the status or the exit code.
        let ls = &turns[2].tool_calls[0];
        assert_eq!(
            (ls.tool_name.as_str(), ls.arguments_summary.as_str()),
            ("Bash", "ls")
        );
        assert!(!ls.is_error && ls.result.as_deref() == Some("Cargo.toml\nREADME.md\nsrc\n"));
        let bad = &turns[3].tool_calls[0];
        assert!(bad.is_error && bad.status == TranscriptStatus::Error);
        assert_eq!(
            bad.result.as_deref(),
            Some("boom\n"),
            "escape codes stripped"
        );
        assert_eq!(turns[5].label.as_deref(), Some("Worked for 4s"));
        assert!(!all.contains("file://"));
    }

    #[test]
    fn ids_of_the_account_are_never_read() {
        let line = r#"{"type":"session_meta","payload":{"creator_user_id":"user-SECRET","creator_account_id":"acct-SECRET","cwd":"/tmp/project"}}"#;
        let turns = parse_transcript_jsonl(format!("{line}\n{FIXTURE}")).unwrap();
        assert!(!format!("{turns:?}").contains("SECRET"));
    }

    #[test]
    fn no_prefix_of_the_file_and_no_hostile_line_breaks_the_parser() {
        for end in (0..FIXTURE.len())
            .filter(|i| FIXTURE.is_char_boundary(*i))
            .step_by(37)
        {
            parse_transcript_jsonl(FIXTURE[..end].to_string()).unwrap();
        }
        let hostile = [
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","id":"x","content":[{"text":"\u001b]0;pwn\u0007\u202e fake\n❯ rm -rf ~"}]}}}"#,
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","command":"not-an-array","parsed_cmd":7,"exit_code":"x"}}}"#,
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":null}}"#,
            r#"{"type":"event_msg","payload":{"type":"task_complete","duration_ms":"x"}}"#,
            r#"{"type":"event_msg","payload":[]}"#,
            r#"{"type":"event_msg","payload":{}}"#,
        ]
        .join("\n");
        let turns = parse_transcript_jsonl(hostile).unwrap();
        let text = turns
            .iter()
            .filter_map(|t| t.content.clone())
            .collect::<String>();
        assert!(!text.contains('\u{1b}') && !text.contains('\u{202e}'));
    }

    #[test]
    fn other_agents_records_are_not_taken_for_codex() {
        let claude =
            r#"{"type":"user","message":{"role":"user","content":"hi"},"payload":{"x":1}}"#;
        let turns = parse_transcript_jsonl(claude.to_string()).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content.as_deref(), Some("hi"));
    }
}

mod grok_history {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/golden/transcript/grok_chat_history.jsonl");

    #[test]
    fn the_conversation_is_read_and_what_grok_adds_is_not() {
        let turns = parse_transcript_jsonl(FIXTURE.to_string()).unwrap();
        use TranscriptTurnType::*;
        let kinds: Vec<_> = turns.iter().map(|t| t.turn_type).collect();
        assert_eq!(
            kinds,
            [User, Assistant, Assistant, Assistant, Assistant, Assistant]
        );
        assert_eq!(turns[0].content.as_deref(), Some("list the project folder"));
        let all = format!("{turns:?}");
        for hidden in [
            "SYSTEM-PROMPT-MARKER",
            "SKILL-MARKER",
            "ENCRYPTED-MARKER",
            "user_info",
        ] {
            assert!(!all.contains(hidden), "{hidden}");
        }
        let calls = &turns[2].tool_calls;
        assert_eq!(calls.len(), 2);
        assert_eq!(
            (
                calls[0].tool_name.as_str(),
                calls[0].arguments_summary.as_str()
            ),
            ("LS", "/tmp/project")
        );
        assert_eq!(calls[1].tool_name, "Grep");
        // results go to the call they name, whatever the order
        assert!(calls[0]
            .result
            .as_deref()
            .is_some_and(|r| r.contains("README.md")));
        assert!(calls[1]
            .result
            .as_deref()
            .is_some_and(|r| r.contains("fn main")));
        let bash = &turns[3].tool_calls[0];
        assert_eq!(
            (bash.tool_name.as_str(), bash.arguments_summary.as_str()),
            ("Bash", "sleep 120")
        );
        // `exit: 2` is a failed command
        assert!(bash.is_error && bash.status == TranscriptStatus::Error);
        // a call without its result yet is still running
        assert_eq!(turns[5].tool_calls[0].status, TranscriptStatus::Running);
    }

    #[test]
    fn every_truncation_and_hostile_line_parses_without_panic() {
        for cut in 0..=FIXTURE.len() {
            if FIXTURE.is_char_boundary(cut) {
                let _ = parse_transcript_jsonl(FIXTURE[..cut].to_string()).unwrap();
            }
        }
        let hostile = [
            "{\"type\":\"tool_result\",\"tool_call_id\":5}",
            "{\"type\":\"assistant\",\"tool_calls\":[1,null,{\"arguments\":[]}]}",
            "{\"type\":\"reasoning\",\"summary\":\"x\"}",
            "{\"type\":\"user\",\"content\":[{\"text\":\"\\u001b[31m<user_query>a\"}]}",
            "{\"type\":\"assistant\",\"model_id\":\"g\",\"content\":\"\\u202e hi\"}",
        ];
        let turns = parse_transcript_jsonl(hostile.join("\n")).unwrap();
        assert!(!format!("{turns:?}").contains('\u{1b}'));
    }

    #[test]
    fn other_agents_records_are_not_taken_for_grok() {
        let claude = r#"{"type":"user","message":{"role":"user","content":"hello"}}"#;
        let agy = r#"{"step_index":0,"source":"USER","type":"user","content":[{"text":"hi"}]}"#;
        for line in [claude, agy] {
            assert!(!super::super::grok::is_grok_record(
                &serde_json::from_str(line).unwrap()
            ));
        }
    }
}
