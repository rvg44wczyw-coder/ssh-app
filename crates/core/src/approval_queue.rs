//! Phone side of the host approval queue (Feature 13): command construction, strict parsing of the
//! host's answers, the request digest and the SSHSIG signature the host-side hook verifies.
//!
//! Everything that comes back from the host is untrusted text: ids are validated, strings are
//! length-capped, and anything that could disguise what the user is approving (control and
//! bidirectional-override characters) is shown as a visible `⟦U+XXXX⟧` marker instead of being
//! rendered. The digest of the *raw* fields must match the host's `command_sha256`, otherwise the
//! entry is dropped.

use crate::error::SshCoreError;
use crate::tmux::shell_quote;
use serde_json::Value;
use sha2::{Digest, Sha256};
use ssh_key::HashAlg;
use zeroize::Zeroizing;

pub const APPROVAL_NAMESPACE: &str = "ssh-app-approval";
pub const MAX_LIST: usize = 20;
const MAX_COMMAND: usize = 2000;
const MAX_FIELD: usize = 512;
const CTL: &str = "python3 -I \"$HOME/.ssh-app/bin/ssh-app-hook\" ctl";

/// What a pending entry asks of the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ApprovalKind {
    /// The host hook waits for allow / deny.
    Approval,
    /// Informational; acknowledged, not decided.
    Notify,
}

/// One request the host hook is waiting on or an informational notice. All text fields are
/// display-safe (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PendingApproval {
    pub id: String,
    pub kind: ApprovalKind,
    pub agent: String,
    pub event: String,
    pub tool: String,
    pub command: String,
    pub cwd: String,
    pub session: String,
    pub timestamp_sec: u64,
    pub expires_sec: u64,
    pub nonce: String,
    pub command_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ApprovalList {
    /// The host's clock; the decision timestamp and the countdown are derived from it.
    pub host_time_sec: u64,
    pub items: Vec<PendingApproval>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ApprovalStatus {
    pub ssh_keygen: bool,
    pub allowed_signers: bool,
    pub pending: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CtlSubcommand {
    List,
    Status,
}

/// `true` for a 32-character lowercase-hex id.
pub(crate) fn is_valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Constant argument vectors only; no data is ever interpolated.
/// `ctl wait <ids>`: a long poll that returns when the pending set differs from `known_ids`
/// (or after the host's own timeout). Ids are validated, so nothing untrusted reaches the shell.
pub(crate) fn build_wait_command(known_ids: &[String]) -> Result<String, SshCoreError> {
    if known_ids.len() > MAX_LIST || known_ids.iter().any(|i| !is_valid_id(i)) {
        return Err(SshCoreError::ApprovalVerificationError {
            reason: "invalid approval id".to_string(),
        });
    }
    let mut ids = known_ids.to_vec();
    ids.sort();
    ids.dedup();
    Ok(format!(
        "sh -c {}",
        shell_quote(&format!("{CTL} wait {}", shell_quote(&ids.join(","))))
    ))
}

pub(crate) fn build_ctl_command(sub: CtlSubcommand) -> String {
    let name = match sub {
        CtlSubcommand::List => "list",
        CtlSubcommand::Status => "status",
    };
    format!("sh -c {}", shell_quote(&format!("{CTL} {name}")))
}

/// Pipes one JSON line (built by us from validated fields) to `ctl <decide|ack>` through stdin.
fn build_ctl_stdin_command(name: &str, json: &str) -> String {
    let inner = format!("printf %s {} | {CTL} {name}", shell_quote(json));
    format!("sh -c {}", shell_quote(&inner))
}

pub(crate) fn build_decide_command(
    id: &str,
    approved: bool,
    ts: u64,
    signature: &str,
) -> Result<String, SshCoreError> {
    if !is_valid_id(id) {
        return Err(SshCoreError::ApprovalVerificationError {
            reason: "invalid approval id".to_string(),
        });
    }
    let json = serde_json::json!({"v": 1, "id": id, "approved": approved, "ts": ts, "signature": signature})
        .to_string();
    Ok(build_ctl_stdin_command("decide", &json))
}

pub(crate) fn build_ack_command(id: &str) -> Result<String, SshCoreError> {
    if !is_valid_id(id) {
        return Err(SshCoreError::ApprovalVerificationError {
            reason: "invalid notification id".to_string(),
        });
    }
    let json = serde_json::json!({ "id": id }).to_string();
    Ok(build_ctl_stdin_command("ack", &json))
}

/// What the user is shown is exactly what is hashed: agent, tool, cwd and command, each as
/// `<utf8 byte length>:<bytes>\n` so no split between fields can collide with another.
pub fn approval_request_digest(agent: &str, tool: &str, cwd: &str, command: &str) -> String {
    let mut h = Sha256::new();
    for field in [agent, tool, cwd, command] {
        h.update(format!("{}:", field.len()).as_bytes());
        h.update(field.as_bytes());
        h.update(b"\n");
    }
    hex::encode(h.finalize())
}

fn is_spoofy(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}

/// Replaces characters that could disguise the text with a visible `⟦U+XXXX⟧` marker.
pub(crate) fn display_safe(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if is_spoofy(c) {
            out.push_str(&format!("⟦U+{:04X}⟧", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

fn str_field<'a>(v: &'a Value, key: &str, max_chars: usize) -> Option<&'a str> {
    let s = v.get(key)?.as_str()?;
    (s.chars().count() <= max_chars).then_some(s)
}

fn parse_item(v: &Value) -> Option<PendingApproval> {
    let id = v.get("id")?.as_str()?;
    if !is_valid_id(id) {
        return None;
    }
    let kind = match v.get("kind")?.as_str()? {
        "approval" => ApprovalKind::Approval,
        "notify" => ApprovalKind::Notify,
        _ => return None,
    };
    let agent = str_field(v, "agent", MAX_FIELD)?;
    let tool = str_field(v, "tool", MAX_FIELD)?;
    let cwd = str_field(v, "cwd", MAX_FIELD)?;
    let command = str_field(v, "command", MAX_COMMAND)?;
    let event = str_field(v, "event", MAX_FIELD)?;
    let session = str_field(v, "session", MAX_FIELD)?;
    let nonce = str_field(v, "nonce", 64)?;
    let hash = str_field(v, "command_sha256", 64)?;
    if !is_hex(hash, 64) || !is_hex(nonce, 32) {
        return None;
    }
    // The raw fields must hash to what the host recorded, or the entry was altered in transit.
    if approval_request_digest(agent, tool, cwd, command) != hash {
        return None;
    }
    Some(PendingApproval {
        id: id.to_string(),
        kind,
        agent: display_safe(agent),
        event: display_safe(event),
        tool: display_safe(tool),
        command: display_safe(command),
        cwd: display_safe(cwd),
        session: display_safe(session),
        timestamp_sec: v.get("ts")?.as_u64()?,
        expires_sec: v.get("expires")?.as_u64()?,
        nonce: nonce.to_string(),
        command_sha256: hash.to_string(),
    })
}

/// Parses `ctl list` output. Anything that is not a well-formed list (queue not installed, an
/// error object, garbage) yields an empty list; malformed entries are dropped individually.
pub(crate) fn parse_approval_list(output: &str) -> ApprovalList {
    let empty = ApprovalList {
        host_time_sec: 0,
        items: Vec::new(),
    };
    let Ok(v) = serde_json::from_str::<Value>(output.trim()) else {
        return empty;
    };
    let (Some(host_time), Some(items)) = (
        v.get("host_time").and_then(Value::as_u64),
        v.get("items").and_then(Value::as_array),
    ) else {
        return empty;
    };
    let mut seen = std::collections::HashSet::new();
    let items = items
        .iter()
        .filter_map(parse_item)
        .filter(|i| seen.insert(i.id.clone()))
        .take(MAX_LIST)
        .collect();
    ApprovalList {
        host_time_sec: host_time,
        items,
    }
}

pub(crate) fn parse_ctl_status(output: &str) -> ApprovalStatus {
    let v: Value = serde_json::from_str(output.trim()).unwrap_or(Value::Null);
    let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    ApprovalStatus {
        ssh_keygen: flag("ssh_keygen"),
        allowed_signers: flag("allowed_signers"),
        pending: v
            .get("pending")
            .and_then(Value::as_u64)
            .map_or(0, |n| n.min(1000) as u32),
    }
}

/// Parses `{"ok":true}` / `{"ok":false,"error":"…"}` from `ctl decide|ack`.
pub(crate) fn parse_ctl_result(output: &str) -> Result<(), SshCoreError> {
    let v: Value = serde_json::from_str(output.trim()).map_err(|_| {
        SshCoreError::ApprovalVerificationError {
            reason: "the host did not answer (is the approval hook installed?)".to_string(),
        }
    })?;
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let reason = v
        .get("error")
        .and_then(Value::as_str)
        .map(|e| display_safe(&e.chars().take(120).collect::<String>()))
        .unwrap_or_else(|| "the host refused the decision".to_string());
    Err(SshCoreError::ApprovalVerificationError { reason })
}

/// Signs `message` with the session key as an OpenSSH SSHSIG (namespace `ssh-app-approval`), the
/// format `ssh-keygen -Y verify` on the host checks.
#[uniffi::export]
pub fn sign_approval_sshsig(
    private_key_openssh: String,
    message: Vec<u8>,
) -> Result<String, SshCoreError> {
    let key_text = Zeroizing::new(private_key_openssh);
    let key = crate::keys::parse_private_key(&key_text)?;
    let sig = key
        .sign(APPROVAL_NAMESPACE, HashAlg::Sha512, &message)
        .map_err(|e| SshCoreError::ApprovalVerificationError {
            reason: format!("signing failed: {e}"),
        })?;
    sig.to_pem(ssh_key::LineEnding::LF)
        .map_err(|e| SshCoreError::ApprovalVerificationError {
            reason: format!("could not encode the signature: {e}"),
        })
}

/// The one `allowed_signers` line the host trusts: our key, restricted to the approval namespace.
pub(crate) fn allowed_signers_line(public_key_openssh: &str) -> Result<String, SshCoreError> {
    let mut parts = public_key_openssh.split_whitespace();
    let (Some(kind), Some(b64)) = (parts.next(), parts.next()) else {
        return Err(SshCoreError::KeyParseError {
            reason: "malformed public key".to_string(),
        });
    };
    let b64_ok = !b64.is_empty()
        && b64
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='));
    if kind != "ssh-ed25519" || !b64_ok {
        return Err(SshCoreError::KeyParseError {
            reason: "only ssh-ed25519 keys can sign approvals".to_string(),
        });
    }
    Ok(format!(
        "ssh-app namespaces=\"{APPROVAL_NAMESPACE}\" {kind} {b64}\n"
    ))
}

/// Host command that creates the queue directories (0700) and installs `allowed_signers` (0600).
pub(crate) fn build_install_key_command(allowed_signers_line: &str) -> String {
    let inner = format!(
        "umask 077; d=\"$HOME/.ssh-app\"; \
         mkdir -p \"$d/run/pending\" \"$d/run/decisions\" && \
         chmod 700 \"$d\" \"$d/run\" \"$d/run/pending\" \"$d/run/decisions\" && \
         printf %s {} > \"$d/allowed_signers.tmp\" && chmod 600 \"$d/allowed_signers.tmp\" && \
         mv -f \"$d/allowed_signers.tmp\" \"$d/allowed_signers\" && echo ok",
        shell_quote(allowed_signers_line)
    );
    format!("sh -c {}", shell_quote(&inner))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_command_sorts_dedups_and_rejects_bad_ids() {
        let a = "a".repeat(32);
        let b = "b".repeat(32);
        let cmd = build_wait_command(&[b.clone(), a.clone(), b.clone()]).unwrap();
        assert!(cmd.contains(&format!(" wait '{a},{b}'")) || cmd.contains(&format!("{a},{b}")));
        assert!(build_wait_command(&[]).is_ok());
        assert!(build_wait_command(&["x; rm -rf ~".to_string()]).is_err());
        assert!(build_wait_command(&["A".repeat(32)]).is_err());
        let many: Vec<String> = (0..=MAX_LIST).map(|i| format!("{i:032x}")).collect();
        assert!(build_wait_command(&many).is_err());
    }

    fn item_json(id: &str, kind: &str, command: &str) -> Value {
        let (agent, tool, cwd) = ("claude", "Bash", "/home/u/proj");
        serde_json::json!({
            "v": 1, "id": id, "kind": kind, "agent": agent, "event": "PermissionRequest",
            "tool": tool, "command": command, "cwd": cwd, "session": "s1", "ts": 100,
            "nonce": "0123456789abcdef0123456789abcdef", "expires": 220,
            "command_sha256": approval_request_digest(agent, tool, cwd, command),
        })
    }

    const ID: &str = "00112233445566778899aabbccddeeff";

    fn list_of(items: Vec<Value>) -> String {
        serde_json::json!({"host_time": 105, "items": items}).to_string()
    }

    #[test]
    fn digest_matches_the_host_algorithm() {
        // Same bytes as the Python hook: "6:claude\n4:Bash\n2:/p\n8:git push\n".
        let expected = {
            let mut h = Sha256::new();
            h.update(b"6:claude\n4:Bash\n2:/p\n8:git push\n");
            hex::encode(h.finalize())
        };
        assert_eq!(
            approval_request_digest("claude", "Bash", "/p", "git push"),
            expected
        );
        // Moving a newline between fields must change the digest (no ambiguous split).
        assert_ne!(
            approval_request_digest("claude", "a\nb", "c", "x"),
            approval_request_digest("claude", "a", "b\nc", "x")
        );
        // Multi-byte text is length-prefixed in bytes, like the hook does.
        assert_ne!(
            approval_request_digest("claude", "Bash", "/é", "x"),
            approval_request_digest("claude", "Bash", "/e", "x")
        );
    }

    #[test]
    fn parses_a_valid_list() {
        let l = parse_approval_list(&list_of(vec![item_json(ID, "approval", "git push")]));
        assert_eq!(l.host_time_sec, 105);
        assert_eq!(l.items.len(), 1);
        let i = &l.items[0];
        assert_eq!(
            (i.id.as_str(), i.kind, i.command.as_str()),
            (ID, ApprovalKind::Approval, "git push")
        );
        assert_eq!((i.timestamp_sec, i.expires_sec), (100, 220));
    }

    #[test]
    fn drops_malformed_and_tampered_entries() {
        let mut tampered = item_json(ID, "approval", "git push");
        tampered["command"] = Value::from("rm -rf ~");
        let mut bad_id = item_json("../../etc/passwd", "approval", "x");
        bad_id["id"] = Value::from("../../etc/passwd");
        let bad_kind = item_json(ID, "other", "x");
        let mut bad_hash = item_json(ID, "approval", "x");
        bad_hash["command_sha256"] = Value::from("zz");
        let mut no_ts = item_json(ID, "approval", "x");
        no_ts.as_object_mut().unwrap().remove("ts");
        let long = item_json(ID, "approval", &"a".repeat(MAX_COMMAND + 1));
        let l = parse_approval_list(&list_of(vec![
            tampered, bad_id, bad_kind, bad_hash, no_ts, long,
        ]));
        assert!(l.items.is_empty(), "{:?}", l.items);
    }

    #[test]
    fn hostile_text_is_made_visible() {
        let cmd = "echo hi\u{202e}evil\u{0}\u{1b}[31m\u{200b}";
        let l = parse_approval_list(&list_of(vec![item_json(ID, "approval", cmd)]));
        let shown = &l.items[0].command;
        assert!(!shown
            .chars()
            .any(|c| matches!(c, '\u{202e}' | '\u{0}' | '\u{1b}' | '\u{200b}')));
        assert!(
            shown.contains("⟦U+202E⟧") && shown.contains("⟦U+001B⟧"),
            "{shown}"
        );
        // Newlines and tabs stay readable.
        let l = parse_approval_list(&list_of(vec![item_json(ID, "approval", "a\n\tb")]));
        assert_eq!(l.items[0].command, "a\n\tb");
    }

    #[test]
    fn caps_the_list_and_dedupes_ids() {
        let mut items = vec![
            item_json(ID, "approval", "a"),
            item_json(ID, "approval", "a"),
        ];
        for n in 0..30u32 {
            items.push(item_json(&format!("{n:032x}"), "notify", "n"));
        }
        let l = parse_approval_list(&list_of(items));
        assert_eq!(l.items.len(), MAX_LIST);
        assert_eq!(l.items.iter().filter(|i| i.id == ID).count(), 1);
    }

    #[test]
    fn garbage_and_error_objects_give_an_empty_list() {
        for s in [
            "",
            "not json",
            "[]",
            r#"{"ok":false,"error":"approval queue is missing"}"#,
            r#"{"host_time":"x","items":[]}"#,
        ] {
            let l = parse_approval_list(s);
            assert!(l.items.is_empty() && l.host_time_sec == 0, "{s}");
        }
    }

    #[test]
    fn json_bombs_from_the_host_are_harmless() {
        let deep = format!("{}{}", "[".repeat(200_000), "]".repeat(200_000));
        assert!(parse_approval_list(&deep).items.is_empty());
        let nested = format!(r#"{{"host_time":1,"items":[{}]}}"#, "[".repeat(100_000));
        assert!(parse_approval_list(&nested).items.is_empty());
        // 200k entries of garbage: bounded work, nothing kept.
        let many = format!(
            r#"{{"host_time":1,"items":[{}]}}"#,
            vec!["{}"; 200_000].join(",")
        );
        assert!(parse_approval_list(&many).items.is_empty());
        assert!(parse_ctl_result(&"{".repeat(100_000)).is_err());
        assert_eq!(parse_ctl_status(&"[".repeat(100_000)).pending, 0);
    }

    #[test]
    fn status_and_result_parsing() {
        let s = parse_ctl_status(
            r#"{"ok":true,"ssh_keygen":true,"allowed_signers":false,"pending":3}"#,
        );
        assert_eq!(
            s,
            ApprovalStatus {
                ssh_keygen: true,
                allowed_signers: false,
                pending: 3
            }
        );
        assert_eq!(
            parse_ctl_status("junk"),
            ApprovalStatus {
                ssh_keygen: false,
                allowed_signers: false,
                pending: 0
            }
        );
        assert!(parse_ctl_result(r#"{"ok":true}"#).is_ok());
        let e = parse_ctl_result("{\"ok\":false,\"error\":\"gone\u{202e}\"}").unwrap_err();
        assert!(format!("{e}").contains("gone⟦U+202E⟧"), "{e}");
        assert!(parse_ctl_result("").is_err());
        assert!(parse_ctl_result(r#"{"ok":false}"#).is_err());
    }

    #[test]
    fn commands_are_constant_or_quoted() {
        assert_eq!(
            build_ctl_command(CtlSubcommand::List),
            "sh -c 'python3 -I \"$HOME/.ssh-app/bin/ssh-app-hook\" ctl list'"
        );
        assert!(build_ctl_command(CtlSubcommand::Status).ends_with("ctl status'"));
        assert!(build_decide_command("bad id; rm -rf ~", true, 1, "sig").is_err());
        assert!(build_ack_command("../x").is_err());
        // A signature containing quotes cannot break out of the printf argument.
        let c =
            build_decide_command(ID, true, 7, "-----BEGIN-----\n'; touch /tmp/pwn; '\n").unwrap();
        assert!(c.starts_with("sh -c '"));
        assert!(c.contains("ctl decide"));
        let c = build_ack_command(ID).unwrap();
        assert!(c.contains("ctl ack") && c.contains(ID));
    }

    #[test]
    fn allowed_signers_line_accepts_only_ed25519() {
        let l = allowed_signers_line("ssh-ed25519 AAAAC3Nz+/= comment here").unwrap();
        assert_eq!(
            l,
            "ssh-app namespaces=\"ssh-app-approval\" ssh-ed25519 AAAAC3Nz+/=\n"
        );
        for bad in [
            "ssh-rsa AAAA",
            "ssh-ed25519",
            "",
            "ssh-ed25519 AA AA\"; evil",
            "ssh-ed25519 A$(id)",
        ] {
            let r = allowed_signers_line(bad);
            if bad.starts_with("ssh-ed25519 AA AA") {
                // Only the first two fields are used, so extra words are dropped, not injected.
                assert_eq!(
                    r.unwrap(),
                    "ssh-app namespaces=\"ssh-app-approval\" ssh-ed25519 AA\n"
                );
            } else {
                assert!(r.is_err(), "{bad}");
            }
        }
    }

    #[test]
    fn install_command_quotes_the_line() {
        let c = build_install_key_command("ssh-app namespaces=\"x\" ssh-ed25519 AAAA\n");
        assert!(c.starts_with("sh -c '") && c.contains("allowed_signers"));
        assert!(c.contains("chmod 700"));
    }

    #[test]
    fn sshsig_roundtrip_with_openssh() {
        use std::process::{Command, Stdio};
        let have = Command::new("ssh-keygen")
            .arg("-?")
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .is_ok();
        if !have {
            return;
        }
        let kp = crate::keys::generate_keypair().unwrap();
        let msg = crate::approval::create_canonical_signing_bytes(
            ID.into(),
            "ab".repeat(32),
            "cd".repeat(16),
            105,
            true,
        );
        let pem = sign_approval_sshsig(kp.private_key_openssh.clone(), msg.clone()).unwrap();
        assert!(pem.starts_with("-----BEGIN SSH SIGNATURE-----"));
        let dir = tempfile::tempdir().unwrap();
        let signers = dir.path().join("allowed_signers");
        std::fs::write(
            &signers,
            allowed_signers_line(&kp.public_key_openssh).unwrap(),
        )
        .unwrap();
        let sigfile = dir.path().join("sig");
        std::fs::write(&sigfile, &pem).unwrap();
        let verify = |message: &[u8], ns: &str| {
            let mut child = Command::new("ssh-keygen")
                .args(["-Y", "verify", "-f"])
                .arg(&signers)
                .args(["-I", "ssh-app", "-n", ns, "-s"])
                .arg(&sigfile)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            use std::io::Write;
            child.stdin.take().unwrap().write_all(message).unwrap();
            child.wait().unwrap().success()
        };
        assert!(verify(&msg, APPROVAL_NAMESPACE));
        assert!(!verify(b"SSH_APP_APPROVAL_V1:tampered", APPROVAL_NAMESPACE));
        assert!(!verify(&msg, "other-namespace"));
    }

    #[test]
    fn signing_rejects_a_bad_key() {
        assert!(sign_approval_sshsig("not a key".into(), b"x".to_vec()).is_err());
    }
}
