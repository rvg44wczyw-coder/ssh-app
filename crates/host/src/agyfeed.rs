//! Antigravity (agy) conversations as the JSON lines the app already understands.
//!
//! agy keeps each conversation in a SQLite database whose `steps` table holds protobuf blobs
//! (no schema is published). This module reads the few fields the feed needs, by field number,
//! and writes them as the Antigravity-format records of the app's transcript parser, so the whole
//! feed pipeline (cursor, stream, `FeedState`) works unchanged. Anything not recognised is skipped,
//! never guessed: a step the decoder does not know simply does not appear.
//!
//! SQLite itself is read through the system `sqlite3` binary, read-only (it handles the WAL).
//! Nothing here writes to agy's files. See `docs/architecture/agy-db-sync-analysis-2026-10-23.md`.

use crate::proc::run_with_input;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const MAX_BLOB: usize = 400 * 1024;
const MAX_FIELDS: usize = 4096;
const MAX_TEXT: usize = 200_000;
const MAX_STEPS: usize = 64;

/// A protobuf field (wire types 0 and 2 are all the decoder needs).
enum Field<'a> {
    Varint,
    Bytes(&'a [u8]),
}

fn varint(buf: &[u8], i: &mut usize) -> Option<u64> {
    let (mut r, mut shift) = (0u64, 0u32);
    loop {
        let b = *buf.get(*i)?;
        *i += 1;
        if shift >= 64 {
            return None;
        }
        r |= u64::from(b & 0x7f) << shift;
        if b < 0x80 {
            return Some(r);
        }
        shift += 7;
    }
}

/// The fields of one message level, in order; `None` if the bytes are not a well-formed message.
fn fields(buf: &[u8]) -> Option<Vec<(u32, Field<'_>)>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        if out.len() >= MAX_FIELDS {
            return None;
        }
        let key = varint(buf, &mut i)?;
        let number = u32::try_from(key >> 3).ok()?;
        match key & 7 {
            0 => {
                varint(buf, &mut i)?;
                out.push((number, Field::Varint));
            }
            2 => {
                let len = usize::try_from(varint(buf, &mut i)?).ok()?;
                let end = i.checked_add(len)?;
                out.push((number, Field::Bytes(buf.get(i..end)?)));
                i = end;
            }
            1 => i = i.checked_add(8).filter(|e| *e <= buf.len())?,
            5 => i = i.checked_add(4).filter(|e| *e <= buf.len())?,
            _ => return None,
        }
    }
    Some(out)
}

fn bytes<'a>(f: &[(u32, Field<'a>)], n: u32) -> Option<&'a [u8]> {
    f.iter().find_map(|(k, v)| match v {
        Field::Bytes(b) if *k == n => Some(*b),
        _ => None,
    })
}

fn sub<'a>(f: &[(u32, Field<'a>)], n: u32) -> Option<Vec<(u32, Field<'a>)>> {
    fields(bytes(f, n)?)
}

/// UTF-8 text without control characters (display text from a private format).
fn text(b: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(b).ok()?;
    let clean: String = s
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .take(MAX_TEXT)
        .collect();
    (!clean.is_empty()).then_some(clean)
}

fn field_text(f: &[(u32, Field<'_>)], n: u32) -> Option<String> {
    text(bytes(f, n)?)
}

/// The status column: 3 done, 7 failed or denied; anything else (9: running or waiting) is not final.
pub fn is_final(status: u32) -> bool {
    matches!(status, 3 | 7)
}

/// One step as an Antigravity-format record, or `None` for a step this decoder does not know.
pub fn step_record(
    idx: u64,
    step_type: u32,
    status: u32,
    payload: &[u8],
    error: &[u8],
) -> Option<Value> {
    let top = fields(payload)?;
    let err_text = fields(error).and_then(|e| {
        e.iter().find_map(|(_, v)| match v {
            Field::Bytes(b) => text(b),
            Field::Varint => None,
        })
    });
    let failed = status == 7;
    match step_type {
        14 => {
            let input = sub(&top, 19)?;
            let content = field_text(&input, 2)?;
            Some(
                json!({"step_index": idx, "source": "USER_EXPLICIT", "type": "USER_INPUT", "content": content}),
            )
        }
        15 => {
            let m = sub(&top, 20)?;
            let mut calls = Vec::new();
            for (k, v) in &m {
                let Field::Bytes(b) = v else { continue };
                if *k != 7 {
                    continue;
                }
                let Some(c) = fields(b) else { continue };
                let name = field_text(&c, 2).unwrap_or_else(|| "tool".to_string());
                let id = field_text(&c, 1).unwrap_or_else(|| format!("call_{idx}_{}", calls.len()));
                let args = field_text(&c, 3)
                    .and_then(|a| serde_json::from_str::<Value>(&a).ok())
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({}));
                calls.push(json!({"id": id, "name": name, "args": args}));
            }
            let content = field_text(&m, 1);
            let thinking = field_text(&m, 3);
            if content.is_none() && thinking.is_none() && calls.is_empty() {
                return None;
            }
            Some(
                json!({"step_index": idx, "source": "MODEL", "type": "PLANNER_RESPONSE",
                "content": content, "thinking": thinking, "tool_calls": calls}),
            )
        }
        132 => {
            let call = sub(&top, 5).and_then(|f| sub(&f, 4));
            let call_id = call.as_ref().and_then(|c| field_text(c, 1));
            let result = sub(&top, 140)
                .and_then(|f| sub(&f, 2))
                .and_then(|f| field_text(&f, 1));
            let content = if failed {
                err_text.or(result)
            } else {
                result.or(err_text)
            }?;
            Some(
                json!({"step_index": idx, "source": "TOOL", "type": "TOOL_RESPONSE", "content": content,
                "tool_call_id": call_id, "status": if failed { "ERROR" } else { "DONE" }}),
            )
        }
        101 => {
            let title = sub(&top, 114)
                .and_then(|n| sub(&n, 2))
                .and_then(|t| field_text(&t, 1))?;
            Some(
                json!({"step_index": idx, "source": "SYSTEM", "type": "SYSTEM_MESSAGE", "content": title}),
            )
        }
        _ => None,
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

pub fn valid_uuid(s: &str) -> bool {
    s.len() == 36
        && s.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit() && !c.is_ascii_uppercase(),
        })
}

fn sqlite(db: &std::path::Path, sql: &str) -> Option<String> {
    let mut cmd = Command::new("sqlite3");
    cmd.arg("-readonly")
        .arg("-separator")
        .arg("\t")
        .arg(db)
        .arg(sql);
    let (code, out) = run_with_input(cmd, b"", Duration::from_secs(10))?;
    (code == Some(0)).then(|| String::from_utf8_lossy(&out).to_string())
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || s.len() / 2 > MAX_BLOB {
        return None;
    }
    hex::decode(s).ok()
}

/// Final steps from index `from`, as JSON lines, with the cursor to continue from: the index of
/// the first step not delivered yet (a running step is waited for, and steps after it too).
pub fn read_steps(db: &std::path::Path, from: u64) -> Option<(u64, String)> {
    // The reader of `sqlite3`'s output keeps at most `MAX_OUT` bytes, and a child that cannot write
    // more would be killed after its timeout. So the sizes are asked first and as many steps are
    // requested as fit in the budget (hex doubles the size); one step that is larger than the
    // budget on its own is skipped.
    let sizes = sqlite(
        db,
        &format!(
            "SELECT min(ifnull(length(step_payload),0),{MAX_BLOB}) * 2 + min(ifnull(length(error_details),0),65536) * 2 + 64 \
             FROM steps WHERE idx >= {from} ORDER BY idx LIMIT {MAX_STEPS};"
        ),
    )?;
    let budget = crate::proc::MAX_OUT as usize - 4096;
    let (mut limit, mut total) = (0usize, 0usize);
    for line in sizes.lines() {
        let size: usize = line.trim().parse().unwrap_or(budget + 1);
        if total + size > budget {
            break;
        }
        total += size;
        limit += 1;
    }
    if limit == 0 {
        // Nothing fits: either no further step, or one too large to read.
        return Some((
            if sizes.lines().next().is_some() {
                from + 1
            } else {
                from
            },
            String::new(),
        ));
    }
    let out = sqlite(
        db,
        &format!(
            "SELECT idx, step_type, status, hex(substr(step_payload,1,{MAX_BLOB})), hex(substr(error_details,1,65536)) \
             FROM steps WHERE idx >= {from} ORDER BY idx LIMIT {limit};"
        ),
    )?;
    let mut cursor = from;
    let mut lines = String::new();
    for row in out.lines() {
        let mut c = row.split('\t');
        let idx: u64 = c.next()?.parse().ok()?;
        let step_type: u32 = c.next()?.parse().ok()?;
        let status: u32 = c.next()?.parse().ok()?;
        if !is_final(status) {
            break;
        }
        let payload = unhex(c.next().unwrap_or(""))?;
        let error = unhex(c.next().unwrap_or("")).unwrap_or_default();
        if let Some(rec) = step_record(idx, step_type, status, &payload, &error) {
            lines.push_str(&rec.to_string());
            lines.push('\n');
        }
        cursor = idx + 1;
    }
    Some((cursor, lines))
}

/// `agy-jsonl <uuid> <from> --raw`: `CUR <next>` on the first line, then the JSON lines (for the
/// stream script, which does not parse JSON). Nothing is printed when the conversation cannot be read.
pub fn jsonl_raw(args: &[String]) -> Option<String> {
    let reply = jsonl_reply(args);
    (reply["ok"] == true).then(|| {
        format!(
            "CUR {}\n{}",
            reply["cursor"],
            reply["jsonl"].as_str().unwrap_or("")
        )
    })
}

/// `agy-conv <cwd> --raw`: just the conversation id, or nothing.
pub fn conv_raw(args: &[String]) -> Option<String> {
    let reply = conv_reply(args);
    reply["conversation"].as_str().map(str::to_string)
}

/// `agy-children <cwd>`: the child conversations (subagents) of the folder's conversation, newest
/// first, at most 20, nesting depth <= 3, from agy's own index. Text fields are cut and cleaned.
pub fn children_reply(args: &[String]) -> Value {
    let Some(uri) = args.first().and_then(|c| workspace_uri(c)) else {
        return json!({"ok": false, "error": "not a plain folder path"});
    };
    let db = home().join(".gemini/antigravity-cli/conversation_summaries.db");
    children_reply_for(&db, &uri)
}

pub fn children_reply_for(db: &std::path::Path, uri: &str) -> Value {
    let parent = conv_reply_for(db, uri);
    let Some(parent_id) = parent["conversation"].as_str() else {
        return json!({"ok": true, "children": []});
    };
    // Text columns travel as hex: they may hold tabs and line breaks.
    let sql = format!(
        "WITH RECURSIVE d(id, depth) AS (\
           SELECT conversation_id, 1 FROM conversation_summaries \
             WHERE parent_conversation_id = '{parent_id}' AND killed = 0 \
           UNION ALL \
           SELECT c.conversation_id, d.depth + 1 FROM conversation_summaries c JOIN d ON c.parent_conversation_id = d.id \
             WHERE d.depth < 3 AND c.killed = 0) \
         SELECT c.conversation_id, hex(substr(c.agent_name, 1, 100)), hex(substr(c.title, 1, 300)), \
                hex(substr(c.preview, 1, 300)), c.step_count, c.status, \
                ifnull(cast(strftime('%s', c.last_modified_time) AS integer), 0), d.depth \
         FROM d JOIN conversation_summaries c ON c.conversation_id = d.id \
         ORDER BY c.last_modified_time DESC LIMIT 20;"
    );
    let Some(out) = sqlite(db, &sql) else {
        return json!({"ok": false, "error": "the index could not be read"});
    };
    let text = |h: &str| -> String {
        unhex(h)
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .take(300)
            .collect()
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let children: Vec<Value> = out
        .lines()
        .filter_map(|row| {
            let c: Vec<&str> = row.split('\t').collect();
            if c.len() < 8 || !valid_uuid(c[0]) {
                return None;
            }
            let title = text(c[2]);
            let title = if title.is_empty() { text(c[3]) } else { title };
            Some(json!({
                "id": c[0],
                "agent": text(c[1]),
                "title": title,
                "steps": c[4].parse::<u64>().unwrap_or(0),
                "running": c[5].ends_with("RUNNING"),
                "mtime": c[6].parse::<u64>().unwrap_or(0),
                "depth": c[7].parse::<u64>().unwrap_or(1),
            }))
        })
        .collect();
    json!({"ok": true, "parent": parent_id, "now": now, "children": children})
}

/// `agy-jsonl <conversation-uuid> <from>`: `from` is the cursor of the last answer (0 at the start).
pub fn jsonl_reply(args: &[String]) -> Value {
    let (Some(id), Some(from)) = (
        args.first(),
        args.get(1).and_then(|a| a.parse::<u64>().ok()),
    ) else {
        return json!({"ok": false, "error": "usage: agy-jsonl <uuid> <from>"});
    };
    if !valid_uuid(id) {
        return json!({"ok": false, "error": "not a conversation id"});
    }
    let db = home().join(format!(".gemini/antigravity-cli/conversations/{id}.db"));
    jsonl_reply_for(&db, from)
}

pub fn jsonl_reply_for(db: &std::path::Path, from: u64) -> Value {
    match read_steps(db, from) {
        Some((cursor, jsonl)) => json!({"ok": true, "cursor": cursor, "jsonl": jsonl}),
        None => json!({"ok": false, "error": "the conversation could not be read"}),
    }
}

/// The folder as it appears in `workspace_uris`: only plain path characters are accepted.
fn workspace_uri(cwd: &str) -> Option<String> {
    if cwd.is_empty()
        || cwd.len() > 1024
        || !cwd.starts_with('/')
        || cwd.contains("..")
        || !cwd
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._- ".contains(c))
    {
        return None;
    }
    Some(format!("file://{}", cwd.replace(' ', "%20")))
}

/// `agy-conv <cwd>`: the conversation of the newest activity in that folder (not killed, not a
/// subagent), from agy's own index.
pub fn conv_reply(args: &[String]) -> Value {
    let Some(uri) = args.first().and_then(|c| workspace_uri(c)) else {
        return json!({"ok": false, "error": "not a plain folder path"});
    };
    let db = home().join(".gemini/antigravity-cli/conversation_summaries.db");
    conv_reply_for(&db, &uri)
}

pub fn conv_reply_for(db: &std::path::Path, uri: &str) -> Value {
    let sql = format!(
        "SELECT conversation_id, step_count, status FROM conversation_summaries \
         WHERE killed = 0 AND nesting_depth = 0 AND instr(workspace_uris, '\"{uri}\"') > 0 \
         ORDER BY last_modified_time DESC LIMIT 1;"
    );
    let Some(out) = sqlite(db, &sql) else {
        return json!({"ok": false, "error": "the index could not be read"});
    };
    let row = out.lines().next().unwrap_or("");
    let mut c = row.split('\t');
    match c.next() {
        Some(id) if valid_uuid(id) => {
            let steps: u64 = c.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let running = c.next().is_some_and(|s| s.ends_with("RUNNING"));
            json!({"ok": true, "conversation": id, "steps": steps, "running": running})
        }
        _ => json!({"ok": true, "conversation": null}),
    }
}
