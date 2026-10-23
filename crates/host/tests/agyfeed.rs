//! The agy decoder on synthetic conversations: databases built here with the system `sqlite3`
//! and protobuf encoded by hand, in a temporary folder. The owner's real agy files are never read.

use serde_json::Value;
use ssh_app_host::agyfeed::{conv_reply_for, jsonl_reply_for, step_record, valid_uuid};
use std::process::Command;

fn varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}
fn s(field: u32, text: &str) -> Vec<u8> {
    m(field, text.as_bytes())
}
fn m(field: u32, body: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    varint(u64::from(field) << 3 | 2, &mut o);
    varint(body.len() as u64, &mut o);
    o.extend_from_slice(body);
    o
}
fn n(field: u32, v: u64) -> Vec<u8> {
    let mut o = Vec::new();
    varint(u64::from(field) << 3, &mut o);
    varint(v, &mut o);
    o
}
fn cat(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.concat()
}

fn user_step(text: &str) -> Vec<u8> {
    cat(&[
        n(1, 14),
        n(4, 3),
        m(19, &cat(&[s(2, text), m(3, &s(1, text))])),
        s(35, "ctx"),
    ])
}
fn model_step(
    answer: Option<&str>,
    thinking: Option<&str>,
    calls: &[(&str, &str, &str)],
) -> Vec<u8> {
    let mut body = Vec::new();
    if let Some(a) = answer {
        body.extend(s(1, a));
    }
    if let Some(t) = thinking {
        body.extend(s(3, t));
    }
    for (id, name, args) in calls {
        body.extend(m(7, &cat(&[s(1, id), s(2, name), s(3, args)])));
    }
    cat(&[n(1, 15), n(4, 3), m(20, &body)])
}
fn tool_step(id: &str, name: &str, args: &str, output: &str) -> Vec<u8> {
    cat(&[
        n(1, 132),
        m(5, &m(4, &cat(&[s(1, id), s(2, name), s(3, args)]))),
        m(140, &m(2, &s(1, output))),
    ])
}

/// (idx, step_type, status, payload, error)
type Row = (u64, u32, u32, Vec<u8>, Vec<u8>);

fn make_db(dir: &std::path::Path, steps: &[Row]) -> std::path::PathBuf {
    let db = dir.join("c.db");
    let mut sql = String::from("CREATE TABLE steps (idx integer, step_type integer NOT NULL DEFAULT 0, status integer NOT NULL DEFAULT 0, step_payload blob, error_details blob, PRIMARY KEY (idx));");
    for (idx, t, st, payload, err) in steps {
        sql.push_str(&format!(
            "INSERT INTO steps VALUES ({idx},{t},{st},X'{}',X'{}');",
            hex::encode(payload),
            hex::encode(err)
        ));
    }
    let out = Command::new("sqlite3")
        .arg(&db)
        .arg(sql)
        .output()
        .expect("sqlite3");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    db
}

fn have_sqlite() -> bool {
    Command::new("sqlite3")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn steps_become_the_records_the_app_parser_reads() {
    let user = step_record(0, 14, 3, &user_step("fix the bug"), b"").expect("user");
    assert_eq!(user["source"], "USER_EXPLICIT");
    assert_eq!(user["content"], "fix the bug");

    let model = step_record(
        1,
        15,
        3,
        &model_step(
            Some("Done."),
            Some("let me think"),
            &[("c1", "run_command", "{\"CommandLine\":\"ls\"}")],
        ),
        b"",
    )
    .expect("model");
    assert_eq!(model["content"], "Done.");
    assert_eq!(model["thinking"], "let me think");
    assert_eq!(model["tool_calls"][0]["name"], "run_command");
    assert_eq!(model["tool_calls"][0]["args"]["CommandLine"], "ls");

    let tool = step_record(
        2,
        132,
        3,
        &tool_step(
            "c1",
            "run_command",
            "{}",
            "Command exited with code 0\na.txt",
        ),
        b"",
    )
    .expect("tool");
    assert_eq!(tool["source"], "TOOL");
    assert_eq!(tool["tool_call_id"], "c1");
    assert!(tool["content"].as_str().unwrap().contains("a.txt"));

    // A denied tool: the error text is the content.
    let denied = step_record(
        3,
        132,
        7,
        &tool_step("c2", "run_command", "{}", ""),
        &s(1, "tool call denied by pre-tool hook"),
    )
    .expect("denied");
    assert_eq!(denied["status"], "ERROR");
    assert!(denied["content"].as_str().unwrap().contains("denied"));

    // Unknown step types and malformed payloads are skipped, not guessed.
    assert!(step_record(4, 17, 3, &cat(&[n(1, 17)]), b"").is_none());
    assert!(step_record(5, 14, 3, &[0xff, 0xff, 0xff], b"").is_none());
    assert!(step_record(6, 15, 3, &cat(&[n(1, 15), m(20, &[])]), b"").is_none());
}

#[test]
fn a_conversation_is_read_in_order_and_waits_for_a_running_step() {
    if !have_sqlite() {
        eprintln!("SKIP: sqlite3 unavailable");
        return;
    }
    let dir = tempfile::tempdir().expect("dir");
    let db = make_db(
        dir.path(),
        &[
            (0, 14, 3, user_step("hello Привет"), vec![]),
            (
                1,
                15,
                3,
                model_step(
                    None,
                    Some("thinking"),
                    &[("c1", "run_command", "{\"CommandLine\":\"ls\"}")],
                ),
                vec![],
            ),
            (
                2,
                132,
                3,
                tool_step("c1", "run_command", "{}", "ok"),
                vec![],
            ),
            (3, 15, 3, model_step(Some("All done"), None, &[]), vec![]),
            (4, 132, 9, tool_step("c2", "run_command", "{}", ""), vec![]),
            (
                5,
                15,
                3,
                model_step(Some("after the running one"), None, &[]),
                vec![],
            ),
        ],
    );
    let r = jsonl_reply_for(&db, 0);
    assert_eq!(r["ok"], true);
    let lines: Vec<Value> = r["jsonl"]
        .as_str()
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        lines
            .iter()
            .map(|l| l["step_index"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(r["cursor"], 4, "the running step 4 holds the cursor back");

    // The same conversation fed to the app's own parser gives user, assistant with thinking, tool result.
    let all = jsonl_reply_for(&db, u64::MAX - 1);
    assert_eq!(all["jsonl"], "");

    // Resuming from the cursor returns nothing until the step finishes.
    let again = jsonl_reply_for(&db, 4);
    assert_eq!(again["jsonl"], "");
    assert_eq!(again["cursor"], 4);
    // From the middle: only the later steps.
    let tail = jsonl_reply_for(&db, 2);
    assert_eq!(tail["jsonl"].as_str().unwrap().lines().count(), 2);
}

#[test]
fn the_conversation_of_a_folder_is_found_through_the_index() {
    if !have_sqlite() {
        return;
    }
    let dir = tempfile::tempdir().expect("dir");
    let db = dir.path().join("cs.db");
    let id1 = "11111111-1111-4111-8111-111111111111";
    let id2 = "22222222-2222-4222-8222-222222222222";
    let id3 = "33333333-3333-4333-8333-333333333333";
    let sql = format!(
        "CREATE TABLE conversation_summaries (conversation_id text, step_count integer, status text, workspace_uris text, last_modified_time text, killed numeric, nesting_depth integer);\
         INSERT INTO conversation_summaries VALUES ('{id1}', 10, 'CASCADE_RUN_STATUS_IDLE', '[\"file:///work/a%20b\"]', '2026-10-01', 0, 0);\
         INSERT INTO conversation_summaries VALUES ('{id2}', 20, 'CASCADE_RUN_STATUS_RUNNING', '[\"file:///work/a%20b\"]', '2026-10-02', 0, 0);\
         INSERT INTO conversation_summaries VALUES ('{id3}', 30, 'CASCADE_RUN_STATUS_IDLE', '[\"file:///work/a%20b\"]', '2026-10-03', 0, 1);"
    );
    assert!(Command::new("sqlite3")
        .arg(&db)
        .arg(sql)
        .status()
        .unwrap()
        .success());
    let r = conv_reply_for(&db, "file:///work/a%20b");
    assert_eq!(r["conversation"], id2, "the newest that is not a subagent");
    assert_eq!(r["running"], true);
    assert_eq!(r["steps"], 20);
    assert_eq!(
        conv_reply_for(&db, "file:///work/other")["conversation"],
        Value::Null
    );
}

#[test]
fn ids_and_folders_are_validated() {
    assert!(valid_uuid("11111111-1111-4111-8111-111111111111"));
    for bad in [
        "",
        "../x",
        "11111111-1111-4111-8111-11111111111",
        "11111111-1111-4111-8111-11111111111G",
        "11111111-1111-4111-8111-1111111111'1",
    ] {
        assert!(!valid_uuid(bad), "{bad}");
    }
    let args = |s: &str| vec![s.to_string()];
    for bad in ["relative", "/a/../b", "/a'b", "/a\"b", "/a;b", "/a$b", ""] {
        assert_eq!(
            ssh_app_host::agyfeed::conv_reply(&args(bad))["ok"],
            false,
            "{bad}"
        );
    }
    assert_eq!(
        ssh_app_host::agyfeed::jsonl_reply(&["../etc".to_string(), "0".to_string()])["ok"],
        false
    );
    assert_eq!(ssh_app_host::agyfeed::jsonl_reply(&[])["ok"], false);
}

#[test]
fn hostile_payloads_do_not_panic_or_blow_up() {
    // Random-looking bytes, truncated lengths, deep nesting, a length that overflows.
    let cases: Vec<Vec<u8>> = vec![
        vec![0x0a, 0xff, 0xff, 0xff, 0xff, 0x0f],
        vec![0xa2, 0x01, 0x05, 1, 2],
        {
            let mut v = Vec::new();
            for _ in 0..10_000 {
                v = m(20, &v);
            }
            v
        },
        vec![0x80; 100],
        vec![0u8; 5000],
    ];
    for payload in cases {
        for t in [14, 15, 101, 132] {
            let _ = step_record(1, t, 3, &payload, &payload);
        }
    }
    // A flood of fields is cut.
    let mut flood = Vec::new();
    for _ in 0..10_000 {
        flood.extend(n(1, 1));
    }
    assert!(step_record(1, 14, 3, &flood, b"").is_none());
}
