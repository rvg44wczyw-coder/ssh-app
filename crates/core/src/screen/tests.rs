#[test]
fn a_path_cut_by_the_terminal_in_the_middle_of_a_word_is_joined_without_a_space() {
    let cut = "  Bash(grep -n x ios/Sources/SSHApp/Views/ActiveTerm"; // fills the pane
    let screen = format!("\n⏺ Bash(grep -n a\n{cut}\n      inalSessionView.swift)\n  ⎿  ok\n");
    let feed = parse_screen_feed(screen);
    let call = &feed
        .turns
        .iter()
        .flat_map(|t| &t.tool_calls)
        .next()
        .unwrap();
    assert!(
        call.arguments_summary
            .contains("ActiveTerminalSessionView.swift"),
        "{}",
        call.arguments_summary
    );
}

#[test]
fn the_auto_mode_note_is_not_a_turn_but_other_notes_stay() {
    let screen = "⏺ Bash(ls)\n  ⎿  a\n\n⏺ Bash(pwd)\n  ⎿  Allowed by auto mode classifier\n\n  ⎿  Stop says: cleanup done\n";
    let feed = parse_screen_feed(screen.into());
    let notes: Vec<_> = feed
        .turns
        .iter()
        .filter(|t| t.label.as_deref() == Some("Note"))
        .collect();
    assert!(notes
        .iter()
        .all(|t| !t.content.clone().unwrap_or_default().contains("auto mode")));
    assert!(feed
        .turns
        .iter()
        .any(|t| t.content.clone().unwrap_or_default().contains("Stop says")));
}

#[test]
fn the_footer_names_the_agent_when_the_scrolled_part_has_no_marker() {
    let agy = "plain text that scrolled by\nmore text\n──────\n>\n──────\n? for shortcuts   accept-edits · Gemini 3.8 Flash · medium\n";
    assert_eq!(parse_screen_feed(agy.into()).dialect, "agy");
    let claude = "plain text\nmore\n  ⏵⏵ auto mode on (shift+tab to cycle)\n";
    assert_eq!(parse_screen_feed(claude.into()).dialect, "claude");
}

#[test]
fn table_rows_with_wrapped_cells_do_not_merge_into_one_line() {
    let screen = "● intro\n   VULN-02:   |    High    |  Low  | cell one\n   Next row   |    Low     |  Low  | cell two\n";
    let feed = parse_screen_feed(screen.into());
    let text = feed
        .turns
        .iter()
        .filter_map(|t| t.content.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("cell one\n"), "{text}");
}

#[test]
fn the_agent_placeholders_for_cut_output_are_not_content() {
    assert!(is_more_marker("… +13 lines"));
    assert!(is_more_marker("<output +73 lines>"));
    assert!(!is_more_marker("<output +x lines>"));
    assert!(!is_more_marker("<output + lines>"));
    assert!(!is_more_marker("output +73 lines"));
}

use super::merge::absorb_user_paragraphs;
use super::*;

fn turns(text: &str) -> ScreenFeed {
    parse_screen_feed(text.to_string())
}

const CLAUDE: &str = "\
 ▐▛███▛█   Claude Code v2.1.303
▝▜██████▀  Sonnet 5.5 · Claude Pro
 ▝▝   ▝▝   ~/rust/ssh-app

❯ в ленте чата только hi, хотя мы с
  тобой уже наговорили

  Searched for 1 pattern (ctrl+o to expand)

⏺ Bash(ls -la ~/.claude/projects 2>&1 | head; env |
      grep -i CLAUDE_CONFIG)
  ⎿  total 0
     drwxr-xr-x@ 19 ivs  staff  608
     … +13 lines (ctrl+o to expand)

⏺ Нашёл причину: лента читала не тот файл. Проверяю
  правку и собираю.

  Причина. Ваш Claude запущен с другим каталогом.
  - первый пункт
  - второй пункт

✻ Churned for 1m 7s · done 6:11 PM

! tmux display -p
  ⎿  Running…

✶ Thinking… (5s · esc to interrupt)
──────────────────────────────────────────────────
❯ черновик
──────────────────────────────────────────────────
  Sonnet 5.5 · 5h 9% · 7d 10%
";

#[test]
fn claude_screen() {
    let f = turns(CLAUDE);
    assert_eq!(f.dialect, "claude");
    assert_eq!(f.draft.as_deref(), Some("черновик"));
    assert_eq!(
        f.status.as_deref(),
        Some("✶ Thinking… (5s · esc to interrupt)")
    );
    let kinds: Vec<String> = f
        .turns
        .iter()
        .map(|t| match (&t.turn_type, &t.label) {
            (TranscriptTurnType::User, _) => "user".to_string(),
            (TranscriptTurnType::System, Some(l)) => format!("sys:{l}"),
            (_, _) if !t.tool_calls.is_empty() => format!("tool:{}", t.tool_calls[0].tool_name),
            _ => "text".to_string(),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "sys:Claude Code",
            "user",
            "tool:Searched for 1 pattern",
            "tool:Bash",
            "text",
            "sys:Worked for 1m 7s",
            "user",
            "sys:Command output",
        ]
    );
    assert_eq!(
        f.turns[1].content.as_deref(),
        Some("в ленте чата только hi, хотя мы с тобой уже наговорили")
    );
    let bash = &f.turns[3].tool_calls[0];
    assert_eq!(
        bash.arguments_summary,
        "ls -la ~/.claude/projects 2>&1 | head; env | grep -i CLAUDE_CONFIG"
    );
    assert!(bash.result.as_deref().unwrap().contains("… +13 lines"));
    let text = f.turns[4].content.as_deref().unwrap();
    assert!(text.starts_with("Нашёл причину"));
    assert!(text.contains("\n\nПричина."));
    assert!(text.contains("- первый пункт\n- второй пункт"));
}

const AGY: &str = "\
      ▄▀▀▄        Antigravity CLI 1.4.2
     ▀▀▀▀▀▀       Gemini 3.8 Flash (Medium)

  Подготовил подробное техническое исследование по
  разработке собственного терминала:

  ### Ключевые выводы

  1. Реализуемость:
      • Задача выполнима.

○ Thought for 6m 29s (Integration tests for `ivs-term...)
● Explored 1 file (lib.rs)
… 95 more actions (23 reads, 25 edits, 47 commands: 7
fai...
● Ran 5 commands (+1 · cargo clipp...) (ctrl+o to expand)

  Проверяю сборку и тесты.

──────────────── Conversation compacted ─────────────────

> давай периодические статусы

⚠ Conversation already open
  ⎿  When you opened this conversation it was already
     open in another CLI instance.

 ┃ Verbosity now defaults to medium
 ┃ esc to dismiss
  ⎿  Interrupted · What should Antigravity CLI do instead?
──────────────────────────────────────────────────────────
>
──────────────────────────────────────────────────────────
? for shortcuts   accept-edits · Gemini 3.8 Flash · medium
";

#[test]
fn agy_screen() {
    let f = turns(AGY);
    assert_eq!(f.dialect, "agy");
    assert_eq!(f.draft, None);
    let kinds: Vec<String> = f
        .turns
        .iter()
        .map(|t| match (&t.turn_type, &t.label) {
            (TranscriptTurnType::User, _) => "user".to_string(),
            (TranscriptTurnType::System, Some(l)) => format!("sys:{l}"),
            (_, _) if t.thinking.is_some() => "thinking".to_string(),
            (_, _) if !t.tool_calls.is_empty() => format!("tool:{}", t.tool_calls[0].tool_name),
            _ => "text".to_string(),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "sys:Antigravity CLI",
            "text",
            "thinking",
            "tool:Explored 1 file",
            "tool:95 more actions",
            "tool:Ran 5 commands",
            "text",
            "sys:Conversation compacted",
            "user",
            "sys:Conversation already open",
            "sys:Notice",
            "sys:Note",
        ]
    );
    assert!(f.turns[1]
        .content
        .as_deref()
        .unwrap()
        .starts_with("Подготовил подробное"));
    assert!(f.turns[4].tool_calls[0]
        .arguments_summary
        .contains("fai..."));
}

#[test]
fn agy_permission_prompt_becomes_a_card() {
    let text = "\
> Как дела? Чем занят?
○ Thought for 6.1s (`uniffi-bindgen` requires a UDL f...)
● Ran 4 commands (2 failed) (ctrl+o to expand)
  Слежу за процессом.
Command
──────────────────────────────────────────────────────────
Requesting permission for:
   file target/debug/libivs_term_bridge.dylib
Run this command?
  1. Yes, run command
  2. Yes, and always allow in this conversation for
commands that start with 'file'
> 3. Yes, and always allow for commands that start with
'file' (Persist to settings.json)
  4. No, cancel
  ↑/↓ Navigate · tab Amend · ctrl+g edit/expand command ·
esc to cancel     accept-edits · Gemini 3.8 Flash · medium
";
    let f = parse_screen_feed(text.to_string());
    let p = f.prompt.expect("a prompt");
    assert_eq!(p.title.as_deref(), Some("Command"));
    assert_eq!(p.detail, "file target/debug/libivs_term_bridge.dylib");
    assert_eq!(p.question, "Run this command?");
    assert_eq!(p.options.len(), 4);
    assert_eq!(p.options[0].label, "Yes, run command");
    assert_eq!(
        p.options[1].label,
        "Yes, and always allow in this conversation for commands that start with 'file'"
    );
    assert_eq!(p.selected, 2);
    assert!(!p.multi);
    // nothing of the menu leaks into the feed
    assert!(f
        .turns
        .iter()
        .all(|t| !t.content.as_deref().unwrap_or("").contains("Yes, run")));
    assert!(f
        .turns
        .iter()
        .any(|t| t.content.as_deref() == Some("Как дела? Чем занят?")));
}

#[test]
fn claude_checkbox_menu_is_multi() {
    let text = "\
⏺ Что включить?

 Что включить?
 ❯ 1. [ ] Хуки
   2. [x] Пуши
   3. [ ] Лента
 Enter to select · ↑/↓ to navigate · Esc to cancel
";
    let f = parse_screen_feed(text.to_string());
    let p = f.prompt.expect("a prompt");
    assert!(p.multi);
    assert_eq!(p.options.len(), 3);
    assert!(p.options[1].checked && !p.options[0].checked);
    assert_eq!(p.selected, 0);
    assert_eq!(p.question, "Что включить?");
}

#[test]
fn claude_question_form_with_tabs_and_descriptions() {
    let text = "\
⏺ Bash(make iphone-full)
  ⎿  App installed

⏺ Теперь задам мультивопрос.
───────────────────────────────────────────────────────────────
←  ☒ Дальше  ☐ Перезапуск  ✔ Submit  →

Как перезапускать агента после включения хуков?

❯ 1. Сам, с подтверждением (Рекомендуется)
     Приложение шлёт /exit и claude --resume
  2. Только подсказка
     Перезапускаете вы
  3. Type something.
───────────────────────────────────────────────────────────────
  4. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel
";
    let f = parse_screen_feed(text.to_string());
    let p = f.prompt.expect("a prompt");
    assert_eq!(
        p.question,
        "Как перезапускать агента после включения хуков?"
    );
    assert_eq!(p.title.as_deref(), Some("☒ Дальше ☐ Перезапуск ✔ Submit"));
    assert_eq!(p.options.len(), 4);
    assert!(p.options[0].label.starts_with("Сам, с подтверждением"));
    assert_eq!(p.options[0].label, "Сам, с подтверждением (Рекомендуется)");
    assert_eq!(
        p.options[0].description,
        "Приложение шлёт /exit и claude --resume"
    );
    assert_eq!(p.options[3].label, "Chat about this");
    assert_eq!(p.selected, 0);
    assert!(f.turns.iter().all(|t| !t
        .content
        .as_deref()
        .unwrap_or("")
        .contains("Перезапускаете")));
}

#[test]
fn claude_form_review_tab_without_hint() {
    let text = "\
❯ Задай мне мультивопрос

──────────────────────────────────────────────────────────
←  ☒ Дальше  ☒ Лента  ✔ Submit  →

Review your answers

 ● Что делать следующим?
   → Хуки в работающей сессии

Ready to submit your answers?

❯ 1. Submit answers
  2. Cancel
";
    let f = parse_screen_feed(text.to_string());
    let p = f.prompt.expect("a prompt");
    assert_eq!(p.question, "Ready to submit your answers?");
    assert_eq!(p.options.len(), 2);
    assert!(p.detail.contains("Хуки в работающей сессии"));
}

#[test]
fn a_running_call_without_its_blinking_bullet_and_a_plain_timer_status_are_read() {
    let screen = "❯ run it\n\n  Bash(S=/tmp/x; tmux display -p\n      '#{pid}' | head)\n  ⎿  Running…\n\n✶ Creating… (4m 30s · ↓ 4.9k tokens)\n\n──────────\n❯ \n──────────\n  Sonnet 5.5 · 5h 1%\n  ⏵⏵ auto mode on (shift+tab to cycle)\n";
    let feed = parse_screen_feed(screen.to_string());
    assert_eq!(
        feed.status.as_deref(),
        Some("✶ Creating… (4m 30s · ↓ 4.9k tokens)")
    );
    let call = feed
        .turns
        .iter()
        .find(|t| !t.tool_calls.is_empty())
        .expect("a tool call");
    assert_eq!(call.tool_calls[0].tool_name, "Bash");
    assert!(
        call.tool_calls[0].arguments_summary.contains("head"),
        "{:?}",
        call.tool_calls[0]
    );
}

#[test]
fn an_expanded_agy_thought_is_split_from_the_reply_by_the_session_file() {
    let screen = "> go\n\n● Read(~/a.md)\n  ⎿  Read 60 lines\n▾ Thought for 25s\n  Analyzing the competitive landscape to identify features.\n  More thinking about the plan here.\n  The answer is forty two and nothing else.\n\n────────────────────\n>\n────────────────────\n? for shortcuts                accept-edits · Gemini 3.8 Flash · medium\n";
    let file = vec![
            TranscriptTurn {
                id: "f1".into(),
                step_index: 1,
                turn_type: TranscriptTurnType::Assistant,
                timestamp: None,
                content: Some("The answer is forty two and nothing else.".into()),
                thinking: Some("Analyzing the competitive landscape to identify features. More thinking about the plan here.".into()),
                tool_calls: vec![],
                status: TranscriptStatus::Done,
                label: None,
            },
        ];
    let feed = parse_screen_feed(screen.to_string());
    let merged = merge_thinking(feed.turns, file);
    let last = merged.last().expect("a reply");
    assert_eq!(
        last.content.as_deref(),
        Some("The answer is forty two and nothing else.")
    );
    assert!(merged.iter().any(|t| t
        .thinking
        .as_deref()
        .is_some_and(|k| k.starts_with("Analyzing"))));
    assert!(merged
        .iter()
        .all(|t| t.label.as_deref() != Some("Thought for 25s")));
}

#[test]
fn a_whole_agy_call_keeps_its_name_and_arguments_apart() {
    let feed = parse_screen_feed("> go\n\n● Bash(ls -la docs)\n  ⎿  Command exited with code 0.\n\n        Command output:\n        a.md\n\n──────\n>\n──────\n".to_string());
    let call = feed
        .turns
        .iter()
        .find_map(|t| t.tool_calls.first())
        .expect("a call");
    assert_eq!(call.tool_name, "Bash");
    assert_eq!(call.arguments_summary, "ls -la docs");
    let note = feed
        .turns
        .iter()
        .find(|t| t.label.as_deref() == Some("Note"))
        .expect("result");
    assert!(note.content.as_deref().unwrap_or("").contains("a.md"));
}

#[test]
fn a_thought_wrapped_back_to_column_zero_stays_one_paragraph() {
    let screen = "> go\n\n● Edited 1 file (a.md) \n▾ Thought for 8s, 1.1k tokens\n  Begin documenting the existing codebase with detailed\nspecifics for someone unfamiliar.\n  \n  Next paragraph of the same thought.\n● Read(~/b.md)\n  ⎿  Read 54 lines\n\n────────────────────\n>\n────────────────────\n? for shortcuts                accept-edits · Gemini 3.8 Flash · medium\n";
    let feed = parse_screen_feed(screen.to_string());
    assert!(
        feed.turns
            .iter()
            .all(|t| t.label.as_deref() != Some("Terminal output")),
        "{:?}",
        feed.turns
            .iter()
            .map(|t| t.label.clone())
            .collect::<Vec<_>>()
    );
    let text = feed
        .turns
        .iter()
        .find(|t| {
            t.content
                .as_deref()
                .is_some_and(|c| c.starts_with("Begin documenting"))
        })
        .expect("the thought text");
    assert!(text
        .content
        .as_deref()
        .unwrap()
        .contains("detailed specifics for someone"));
    // the header after a folded group line is a marker of its own, not part of the group
    assert!(feed.turns.iter().any(|t| t
        .label
        .as_deref()
        .is_some_and(|l| l.starts_with("Thought for 8s"))));
}

#[test]
fn the_screen_is_cleaned_of_escapes_and_control_characters() {
    let dirty = "\u{1b}]0;title\u{7}❯ hi\u{1b}[31m red\u{8}\u{202e}evil\u{7}\n";
    let feed = parse_screen_feed(dirty.to_string());
    let text = format!("{feed:?}");
    for bad in ['\u{1b}', '\u{7}', '\u{8}', '\u{202e}'] {
        assert!(!text.contains(bad), "{bad:?} survived: {text}");
    }
}

fn user(text: &str) -> TranscriptTurn {
    TranscriptTurn {
        id: text.into(),
        step_index: 0,
        turn_type: TranscriptTurnType::User,
        timestamp: None,
        content: Some(text.into()),
        thinking: None,
        tool_calls: vec![],
        status: TranscriptStatus::Done,
        label: None,
    }
}

fn assistant_text(text: &str) -> TranscriptTurn {
    TranscriptTurn {
        turn_type: TranscriptTurnType::Assistant,
        ..user(text)
    }
}

#[test]
fn paragraphs_of_your_message_drawn_as_a_reply_go_back_into_it() {
    let screen = vec![
        user("напрямую через Tailscale сокет."),
        assistant_text("Не подходит, ts тут чисто сети."),
        assistant_text("A real reply."),
    ];
    let file = vec![user(
        "напрямую через Tailscale сокет.\n\nНе подходит, ts тут чисто сети.",
    )];
    let merged = absorb_user_paragraphs(screen.clone(), &file);
    assert_eq!(merged.len(), 2);
    assert_eq!(
        merged[0].content.as_deref(),
        Some("напрямую через Tailscale сокет.\n\nНе подходит, ts тут чисто сети.")
    );
    assert_eq!(merged[1].content.as_deref(), Some("A real reply."));
    // a reply that merely follows the message is left alone
    let alone = absorb_user_paragraphs(screen, &[user("напрямую через Tailscale сокет.")]);
    assert_eq!(alone.len(), 3);
}

#[test]
fn a_file_matches_the_screen_when_something_you_said_is_on_it() {
    let screen = "❯ fix the login\n\n⏺ Done.\n❯ and   now the\n  tests please\n".to_string();
    assert!(transcript_matches_screen(
        vec![user("fix the login"), user("and now the tests please")],
        screen.clone()
    ));
    // another conversation in the same folder
    assert!(!transcript_matches_screen(
        vec![user("rewrite the parser")],
        screen.clone()
    ));
    // nothing said yet: cannot tell, accepted
    assert!(transcript_matches_screen(vec![], screen));
}

#[test]
fn numbered_reply_above_a_working_footer_is_not_a_menu() {
    let text = "\
> Опиши варианты
● Вот варианты:
  1. Первый вариант
  2. Второй вариант
  Первый лучше, потому что проще.
  Второй дороже в поддержке.
  Итого рекомендую первый.
  Это описание, а не выбор.
  Спрашивайте дальше.
────────────────────────────────────────
>
────────────────────────────────────────
  esc to cancel     accept-edits · Gemini 3.8 Flash · medium
";
    let f = parse_screen_feed(text.to_string());
    assert!(f.prompt.is_none());
}

#[test]
fn folded_group_gets_its_calls_from_the_file() {
    let mk = |content: Option<&str>, tool: Option<(&str, &str, Option<&str>)>| TranscriptTurn {
        id: "x".into(),
        step_index: 0,
        turn_type: TranscriptTurnType::Assistant,
        timestamp: None,
        content: content.map(String::from),
        thinking: None,
        tool_calls: tool
            .map(|(n, a, r)| TranscriptToolCall {
                call_id: "c".into(),
                tool_name: n.into(),
                arguments_summary: a.into(),
                full_arguments_json: None,
                result: r.map(String::from),
                is_error: false,
                status: TranscriptStatus::Done,
            })
            .into_iter()
            .collect(),
        status: TranscriptStatus::Done,
        label: None,
    };
    let screen = vec![
        mk(None, Some(("Searched for 1 pattern", "", None))),
        mk(Some("Нашёл причину: лента читала не тот файл."), None),
    ];
    let file = vec![
        mk(
            None,
            Some((
                "Grep",
                "pattern: projects",
                Some("a.rs:1\nb.rs:2\nc.rs:3\nd.rs:4"),
            )),
        ),
        mk(Some("Нашёл причину: лента читала не тот файл."), None),
    ];
    let out = merge_thinking(screen, file);
    let c = &out[0].tool_calls[0];
    assert_eq!(c.tool_name, "Searched for 1 pattern");
    assert_eq!(c.arguments_summary, "pattern: projects");
    let r = c.result.as_deref().unwrap();
    assert!(r.starts_with("Grep(pattern: projects)\n  a.rs:1"));
    assert!(
        r.ends_with("  d.rs:4"),
        "the whole result is shown, not its first lines: {r}"
    );
}

#[test]
fn results_are_filtered_for_noise_not_cut() {
    let result = "\n  Compiling a v1\n  Compiling b v2\nCompiling c v3\nwarning: unused\n\n… +20 lines (ctrl+o to expand)\n… +5 lines\nerror: boom\n";
    assert_eq!(
        readable_result_lines(result),
        [
            "Compiling a v1 (+2 similar lines)".to_string(),
            "warning: unused".into(),
            "error: boom".into()
        ]
    );
    assert_eq!(readable_result_lines("Compiling only"), ["Compiling only"]);
    assert!(readable_result_lines("\n \n").is_empty());
    // Ordinary lines that merely look similar stay.
    assert_eq!(readable_result_lines("… +x lines"), ["… +x lines"]);
}

#[test]
fn a_long_result_in_a_folded_group_keeps_forty_lines_and_says_how_many_more() {
    let mut g = TranscriptToolCall {
        call_id: "g".into(),
        tool_name: "Ran 2 commands".into(),
        arguments_summary: String::new(),
        full_arguments_json: None,
        result: None,
        is_error: false,
        status: TranscriptStatus::Done,
    };
    let long = (1..=55)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let wide = "x".repeat(900);
    let base = g.clone();
    let call = |r: &str| TranscriptToolCall {
        tool_name: "Bash".into(),
        arguments_summary: "cmd".into(),
        result: Some(r.into()),
        ..base.clone()
    };
    let calls = [call(&long), call(&wide)];
    describe_group(&mut g, &calls);
    let r = g.result.unwrap();
    assert!(r.contains("  line 40\n") && !r.contains("line 41"));
    assert!(r.contains("  … +15 lines"));
    assert!(
        r.lines()
            .any(|l| l.chars().count() == 2 + GROUP_LINE_CHARS + 1),
        "a wide line is clipped at 400"
    );
}

#[test]
fn matched_edit_gets_arguments_and_result_from_the_file() {
    let call = |args: Option<&str>, result: &str| TranscriptToolCall {
        call_id: "c".into(),
        tool_name: "Edit".into(),
        arguments_summary: "a.rs".into(),
        full_arguments_json: args.map(String::from),
        result: Some(result.into()),
        is_error: false,
        status: TranscriptStatus::Done,
    };
    let turn = |c| TranscriptTurn {
        id: "x".into(),
        step_index: 0,
        turn_type: TranscriptTurnType::Assistant,
        timestamp: None,
        content: None,
        thinking: None,
        tool_calls: vec![c],
        status: TranscriptStatus::Done,
        label: None,
    };
    let screen = vec![turn(call(
        None,
        "Added 2 lines\n… +2 lines (ctrl+o to expand)",
    ))];
    let file = vec![turn(call(Some("{\"old_string\":\"a\"}"), "full result"))];
    let out = merge_thinking(screen, file);
    let c = &out[0].tool_calls[0];
    assert_eq!(
        c.full_arguments_json.as_deref(),
        Some("{\"old_string\":\"a\"}")
    );
    assert_eq!(c.result.as_deref(), Some("full result"));
}

#[test]
fn ctrl_o_transcript_view_keeps_full_calls_and_drops_its_footer() {
    let text = include_str!("../../tests/data/claude_ctrl_o.txt");
    let f = parse_screen_feed(text.to_string());
    assert_eq!(f.dialect, "claude");
    let bash = f
        .turns
        .iter()
        .flat_map(|t| t.tool_calls.iter())
        .find(|c| c.tool_name == "Bash")
        .expect("a Bash call");
    assert!(bash.arguments_summary.contains("python3"));
    assert!(bash
        .result
        .as_deref()
        .unwrap_or("")
        .contains("test result: ok"));
    assert!(f.turns.iter().all(|t| !t
        .content
        .as_deref()
        .unwrap_or("")
        .contains("Showing detailed transcript")));
    assert!(f.prompt.is_none());
}

#[test]
fn no_prompt_without_a_hint_line() {
    let f = parse_screen_feed("⏺ Вот план:\n\n  1. Первое\n  2. Второе\n".to_string());
    assert!(f.prompt.is_none());
    assert_eq!(f.turns.len(), 1);
}

#[test]
fn unknown_screen_is_kept_folded() {
    let f = turns("user@host ~ % ls\nfoo bar\n");
    assert_eq!(f.dialect, "unknown");
    assert_eq!(f.turns.len(), 1);
    assert!(f.turns[0].content.as_deref().unwrap().contains("foo bar"));
}

#[test]
fn thinking_is_put_before_its_reply_and_tool() {
    let screen = parse_screen_feed(CLAUDE.to_string()).turns;
    let mk = |thinking: Option<&str>, content: Option<&str>, tool: Option<(&str, &str)>| {
        TranscriptTurn {
            id: "f".into(),
            step_index: 0,
            turn_type: TranscriptTurnType::Assistant,
            timestamp: None,
            content: content.map(String::from),
            thinking: thinking.map(String::from),
            tool_calls: tool
                .map(|(n, a)| {
                    vec![TranscriptToolCall {
                        call_id: "c".into(),
                        tool_name: n.into(),
                        arguments_summary: a.into(),
                        full_arguments_json: None,
                        result: None,
                        is_error: false,
                        status: TranscriptStatus::Done,
                    }]
                })
                .unwrap_or_default(),
            status: TranscriptStatus::Done,
            label: None,
        }
    };
    let file = vec![
        mk(
            Some("first thought"),
            None,
            Some(("Bash", "ls -la ~/.claude/projects 2>&1 | head")),
        ),
        mk(
            Some("second thought"),
            Some("Нашёл причину: лента читала не тот файл. Проверяю правку и собираю."),
            None,
        ),
    ];
    let merged = merge_thinking(screen.clone(), file);
    assert_eq!(merged.len(), screen.len() + 2);
    let pos = |pred: &dyn Fn(&TranscriptTurn) -> bool| merged.iter().position(pred).unwrap();
    let t1 = pos(&|t| t.thinking.as_deref() == Some("first thought"));
    let bash = pos(&|t| t.tool_calls.first().is_some_and(|c| c.tool_name == "Bash"));
    let t2 = pos(&|t| t.thinking.as_deref() == Some("second thought"));
    let reply = pos(&|t| {
        t.content
            .as_deref()
            .is_some_and(|c| c.starts_with("Нашёл причину"))
    });
    assert_eq!(t1 + 1, bash);
    assert_eq!(t2 + 1, reply);
    assert!(bash < t2);
}
