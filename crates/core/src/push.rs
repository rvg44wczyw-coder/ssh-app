//! Push registration of the phone on a host (feature 35, phase T3): the device token, the relay's
//! address and its secret go to the pinned helper (`push-register`), which keeps them in a private
//! file and lets the host agent ask the relay for one of four constant notifications.

use crate::error::SshCoreError;
use crate::hostprobe::{login_sh_script, SHA256_OF_H};
use crate::tmux::shell_quote;
use crate::usage::{validate_sha256, HELPER_PATH_EXPR};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostPushStatus {
    pub registered: bool,
    pub env: String,
    pub relay: String,
    pub token_tail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostPushResult {
    pub ok: bool,
    /// A short reason, empty when `ok`.
    pub message: String,
}

fn bad(what: &str) -> SshCoreError {
    crate::agents::invalid(what)
}

/// `https://host[:port]/push`, a plain dotted host name and nothing else.
pub fn validate_relay_url(u: &str) -> Result<(), SshCoreError> {
    let host = u
        .strip_prefix("https://")
        .and_then(|r| r.strip_suffix("/push"))
        .ok_or_else(|| bad("The relay address must look like https://name.example.com/push"))?;
    let (name, port) = host
        .split_once(':')
        .map_or((host, None), |(n, p)| (n, Some(p)));
    let ok = u.len() <= 200
        && name.contains('.')
        && !name.starts_with(['.', '-'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && port
            .is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()));
    if ok {
        Ok(())
    } else {
        Err(bad("The relay address is not valid"))
    }
}

pub fn validate_relay_secret(s: &str) -> Result<(), SshCoreError> {
    if (32..=128).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Ok(())
    } else {
        Err(bad(
            "The relay secret must be 32 to 128 letters, digits, - or _",
        ))
    }
}

fn validate_token(t: &str) -> Result<(), SshCoreError> {
    if t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(bad("Not a device token"))
    }
}

/// Runs a helper subcommand when the helper is the pinned build. `input` is piped to its stdin.
fn command(sha256: &str, sub: &str, input: Option<&str>) -> String {
    let feed = if input.is_some() {
        "printf '%s' \"$1\" | "
    } else {
        ""
    };
    let script = format!(
        "h={HELPER_PATH_EXPR}; if [ ! -f \"$h\" ] || [ -L \"$h\" ] || \
         [ \"{SHA256_OF_H}\" != '{sha256}' ]; then echo HELPER_ABSENT; \
         else {feed}\"$h\" {sub} 2>/dev/null; fi"
    );
    let args: Vec<String> = input.map(shell_quote).into_iter().collect();
    login_sh_script(&script, &args, 20)
}

pub(crate) fn build_register_command(
    sha256: &str,
    token: &str,
    env: &str,
    url: &str,
    secret: &str,
) -> Result<String, SshCoreError> {
    validate_sha256(sha256)?;
    validate_token(token)?;
    if env != "production" && env != "sandbox" {
        return Err(bad("Unknown push environment"));
    }
    validate_relay_url(url)?;
    validate_relay_secret(secret)?;
    let json =
        serde_json::json!({"token": token, "env": env, "url": url, "secret": secret}).to_string();
    Ok(command(sha256, "push-register", Some(&json)))
}

pub(crate) fn build_push_command(sha256: &str, sub: &str) -> Result<String, SshCoreError> {
    validate_sha256(sha256)?;
    debug_assert!(matches!(
        sub,
        "push-status" | "push-unregister" | "push-test"
    ));
    Ok(command(sha256, sub, None))
}

fn reply(out: &str) -> Option<serde_json::Value> {
    out.lines()
        .rev()
        .find_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok())
        .filter(serde_json::Value::is_object)
}

fn short(v: &serde_json::Value, k: &str, max: usize) -> String {
    v.get(k)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        .take(max)
        .collect()
}

pub(crate) fn parse_status(out: &str) -> Option<HostPushStatus> {
    let v = reply(out)?;
    if v.get("ok") != Some(&true.into()) {
        return None;
    }
    Some(HostPushStatus {
        registered: v.get("registered") == Some(&true.into()),
        env: short(&v, "env", 16),
        relay: short(&v, "relay", 100),
        token_tail: short(&v, "token_tail", 6),
    })
}

pub(crate) fn parse_result(out: &str) -> HostPushResult {
    if out.lines().any(|l| l.trim() == "HELPER_ABSENT") {
        return HostPushResult {
            ok: false,
            message: "The host helper is not installed or is another version".into(),
        };
    }
    match reply(out) {
        Some(v) if v.get("ok") == Some(&true.into()) => HostPushResult {
            ok: true,
            message: String::new(),
        },
        Some(v) => HostPushResult {
            ok: false,
            message: v
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("failed")
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '-' | ','))
                .take(80)
                .collect(),
        },
        None => HostPushResult {
            ok: false,
            message: "No answer from the host".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const TOKEN: &str = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";
    const SECRET: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
    const URL: &str = "https://relay.example.workers.dev/push";

    #[test]
    fn arguments_are_validated_before_a_command_is_built() {
        assert!(build_register_command(SHA, TOKEN, "sandbox", URL, SECRET).is_ok());
        for (t, e, u, s) in [
            ("zz", "sandbox", URL, SECRET),
            (TOKEN, "dev", URL, SECRET),
            (TOKEN, "sandbox", "http://a.b/push", SECRET),
            (TOKEN, "sandbox", "https://a.b/push'; rm -rf ~; '", SECRET),
            (TOKEN, "sandbox", "https://a.b/other", SECRET),
            (TOKEN, "sandbox", URL, "short"),
            (
                TOKEN,
                "sandbox",
                URL,
                "0123456789abcdef0123456789abcdef'; id; '",
            ),
        ] {
            assert!(
                build_register_command(SHA, t, e, u, s).is_err(),
                "{t} {e} {u} {s}"
            );
        }
        assert!(build_register_command("nothex", TOKEN, "sandbox", URL, SECRET).is_err());
    }

    #[test]
    fn the_registration_travels_as_one_quoted_argument_to_the_pinned_helper() {
        let c = build_register_command(SHA, TOKEN, "sandbox", URL, SECRET).unwrap();
        assert!(c.contains("push-register"));
        assert!(c.contains(SHA));
        assert!(c.contains("HELPER_ABSENT"));
        assert!(
            !c.contains("push-register '"),
            "the secret is not an argument of the helper"
        );
        let s = build_push_command(SHA, "push-status").unwrap();
        assert!(s.contains("push-status") && !s.contains(SECRET));
    }

    #[test]
    fn replies_are_parsed_and_sanitised() {
        let st = parse_status(
            "{\"ok\":true,\"registered\":true,\"env\":\"sandbox\",\"relay\":\"a.b\",\"token_tail\":\"cdcdcd\"}\n",
        )
        .unwrap();
        assert!(st.registered && st.env == "sandbox" && st.token_tail == "cdcdcd");
        assert!(
            !parse_status("{\"ok\":true,\"registered\":false}")
                .unwrap()
                .registered
        );
        assert!(parse_status("HELPER_ABSENT").is_none());
        assert!(parse_result("{\"ok\":true}").ok);
        assert!(parse_result("HELPER_ABSENT").message.contains("helper"));
        let r = parse_result("{\"ok\":false,\"error\":\"relay failed\\u001b[31m\"}");
        assert!(!r.ok && !r.message.contains('\u{1b}'));
        assert!(!parse_result("garbage").ok);
    }
}
