//! The agent feed on Zellij (feature 49, milestone B): shell pieces that find the focused terminal
//! pane of a session, dump its screen and find the processes that run in it. tmux gives a pane pid
//! for the last two; Zellij does not, so a process is recognised by the environment every process
//! in a pane carries (`ZELLIJ_SESSION_NAME`, `ZELLIJ_PANE_ID`). Names reach these scripts as
//! validated, single-quoted arguments and awk sees them only as `-v` variables.

use crate::error::SshCoreError;
use crate::multiplexer::MultiplexerKind;

pub(crate) const PATH_FIX: &str = r#"PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"; export PATH"#;

/// `zj_pane NAME` prints the number of the focused terminal pane of the session: the first
/// `terminal_N` row of `list-panes -s` (the flag adds the state columns) whose third-from-last token (`FOCUSED FLOATING EXITED`) is
/// `true`. stderr is dropped: Zellij's "not found" text lists the other sessions.
/// `zj_roots NAME N` prints the pids whose environment has both the session name and the pane id
/// (macOS: `ps -Eww`; elsewhere `/proc/<pid>/environ`). Only pids are printed, never environment text.
pub(crate) const ZELLIJ_FUNCTIONS: &str = r#"zj_pane() {
  zellij --session "$1" action list-panes -s 2>/dev/null | awk 'NF >= 4 && $1 ~ /^terminal_[0-9]+$/ && $(NF-2) == "true" { sub(/^terminal_/, "", $1); print $1; exit }'
}
zj_roots() {
  if [ -d /proc/self ]; then
    for d in /proc/[0-9]*; do
      [ -r "$d/environ" ] || continue
      if tr '\0' '\n' < "$d/environ" 2>/dev/null | awk -v s="ZELLIJ_SESSION_NAME=$1" -v p="ZELLIJ_PANE_ID=$2" '$0 == s { a = 1 } $0 == p { b = 1 } END { exit !(a && b) }'; then echo "${d#/proc/}"; fi
    done
  else
    ps -Eww -Ao pid=,command= 2>/dev/null | awk -v s="ZELLIJ_SESSION_NAME=$1" -v p="ZELLIJ_PANE_ID=$2" '{ a = 0; b = 0; for (i = 2; i <= NF; i++) { if ($i == s) a = 1; if ($i == p) b = 1 } if (a && b) print $1 }'
  fi
}
"#;

/// The text of the focused pane with its scrollback and styles, in the `SAME`/`SUM` protocol of
/// the tmux capture. A release that refuses `--ansi` is asked again without it.
const ZELLIJ_CAPTURE_BODY: &str = r#"n=$(zj_pane "$1"); [ -n "$n" ] || exit 1
out=$(zellij --session "$1" action dump-screen --pane-id "terminal_$n" --full --ansi 2>/dev/null) || out=$(zellij --session "$1" action dump-screen --pane-id "terminal_$n" --full 2>/dev/null) || exit 1
sum=$(printf '%s' "$out" | cksum | cut -d' ' -f1)
if [ "$sum" = "$2" ]; then echo "SAME $sum"; else echo "SUM $sum"; printf '%s\n' "$out"; fi
exit 0"#;

pub(crate) fn zellij_capture_script() -> String {
    format!("{PATH_FIX}\n{ZELLIJ_FUNCTIONS}{ZELLIJ_CAPTURE_BODY}")
}

/// Shell prefix that sets `roots` (space-separated pids) for the pane process walk of either
/// program; `$2` of the calling script is the program (`tmux` or `zellij`). It exits when the
/// session or pane is gone. `on_missing` is the shell text run before that exit.
pub(crate) fn roots_prefix(on_missing: &str) -> String {
    format!(
        r#"{PATH_FIX}
{ZELLIJ_FUNCTIONS}if [ "$2" = zellij ]; then
  n=$(zj_pane "$1")
  [ -n "$n" ] || {{ {on_missing} exit 0; }}
  roots=$(zj_roots "$1" "$n" | tr '\n' ' ')
  case "$roots" in *[!0-9\ ]*) {on_missing} exit 0;; esac
else
  roots=$(tmux display-message -p -t "=$1:" '#{{pane_pid}}' 2>/dev/null)
  case "$roots" in ""|*[!0-9]*) {on_missing} exit 0;; esac
fi
"#
    )
}

/// `tmux` / `zellij` as the scripts take it in `$2`, after the name is checked for that program.
pub(crate) fn mux_arg(kind: MultiplexerKind, name: &str) -> Result<&'static str, SshCoreError> {
    kind.validate_name(name)?;
    match kind {
        MultiplexerKind::Tmux => Ok("tmux"),
        MultiplexerKind::Zellij => Ok("zellij"),
        MultiplexerKind::WindowsMux => Err(SshCoreError::Unsupported {
            reason: "The agent feed needs tmux or Zellij on the host".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn awk_pane(table: &str) -> String {
        let prog = r#"NF >= 4 && $1 ~ /^terminal_[0-9]+$/ && $(NF-2) == "true" { sub(/^terminal_/, "", $1); print $1; exit }"#;
        let mut c = Command::new("awk")
            .arg(prog)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("awk");
        use std::io::Write;
        c.stdin
            .take()
            .expect("stdin")
            .write_all(table.as_bytes())
            .expect("write");
        String::from_utf8_lossy(&c.wait_with_output().expect("out").stdout)
            .trim()
            .to_string()
    }

    #[test]
    fn the_functions_text_carries_the_awk_program_the_test_runs() {
        assert!(ZELLIJ_FUNCTIONS.contains(r#"$(NF-2) == "true""#));
    }

    #[test]
    fn the_focused_terminal_is_found_even_when_a_plugin_has_focus() {
        let t = "PANE_ID  TYPE  TITLE  COMMAND  CWD  FOCUSED  FLOATING  EXITED\n\
                 plugin_0  plugin  (.) - zellij:link  zellij:link  -  false  false  false\n\
                 plugin_3  plugin  First Run Setup Wizard (Step 1/1)  configuration  -  true  true  false\n\
                 terminal_0  terminal  Pane #1  /bin/zsh  /tmp/a b  false  false  false\n\
                 terminal_4  terminal  My pane  /bin/zsh  /tmp  true  false  false\n";
        assert_eq!(awk_pane(t), "4");
    }

    #[test]
    fn no_focused_terminal_gives_nothing() {
        assert_eq!(awk_pane("PANE_ID TYPE TITLE COMMAND CWD FOCUSED FLOATING EXITED\nterminal_1 terminal t c d false false false\n"), "");
        assert_eq!(awk_pane(""), "");
        assert_eq!(awk_pane("terminal_1 true\n"), "");
    }

    #[test]
    fn scripts_name_the_session_only_through_arguments() {
        let s = zellij_capture_script();
        assert!(s.contains(r#"--session "$1""#));
        assert!(!s.contains("{{") && !s.contains("}}"));
        let p = roots_prefix("echo X;");
        assert!(p.contains(r#"[ "$2" = zellij ]"#));
        assert!(p.contains("#{pane_pid}"));
        assert!(p.contains("echo X; exit 0"));
    }

    #[test]
    fn the_program_is_checked_with_the_name_rules_of_that_program() {
        assert_eq!(
            mux_arg(MultiplexerKind::Tmux, "claude-code").ok(),
            Some("tmux")
        );
        assert_eq!(
            mux_arg(MultiplexerKind::Zellij, "a-b_1").ok(),
            Some("zellij")
        );
        assert!(mux_arg(MultiplexerKind::Zellij, "a.b").is_err());
        assert!(mux_arg(MultiplexerKind::Zellij, "a b").is_err());
        assert!(matches!(
            mux_arg(MultiplexerKind::WindowsMux, "abc"),
            Err(SshCoreError::Unsupported { .. })
        ));
    }

    /// Against the real `zellij` when the machine has one: a private socket folder, never the
    /// developer's sessions (`ZELLIJ`, `ZELLIJ_SESSION_NAME` and `TMUX` removed).
    #[test]
    fn the_feed_scripts_work_against_a_private_zellij() {
        if Command::new("zellij").arg("--version").output().is_err() {
            eprintln!("SKIP: zellij is not installed");
            return;
        }
        // short path: the socket path of a Unix socket is limited to about 100 bytes
        let dir = std::path::PathBuf::from(format!("/tmp/zf-{}", rand::random::<u32>()));
        std::fs::create_dir_all(&dir).expect("dir");
        // Stops the private server and removes the folder even when an assertion fails.
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = Command::new("sh")
                    .arg("-c")
                    .arg(format!(
                        "{PATH_FIX}; zellij kill-all-sessions -y; zellij delete-all-sessions -y -f"
                    ))
                    .env("ZELLIJ_SOCKET_DIR", &self.0)
                    .env("XDG_RUNTIME_DIR", &self.0)
                    .env_remove("ZELLIJ")
                    .env_remove("ZELLIJ_SESSION_NAME")
                    .env_remove("TMUX")
                    .output();
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("chmod");
        let sh = |script: &str, args: &[&str]| {
            let out = Command::new("sh")
                .arg("-c")
                .arg(format!("{PATH_FIX}\n{script}"))
                .arg("sh")
                .args(args)
                .env("SHELL", "/bin/sh")
                .env("ZELLIJ_SOCKET_DIR", &dir)
                .env("XDG_RUNTIME_DIR", &dir)
                .env_remove("ZELLIJ")
                .env_remove("ZELLIJ_SESSION_NAME")
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .output()
                .expect("sh");
            (
                out.status.success(),
                String::from_utf8_lossy(&out.stdout).into_owned(),
            )
        };
        let wait = |what: &str, f: &dyn Fn() -> bool| {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while std::time::Instant::now() < end {
                if f() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            panic!("timed out: {what}");
        };
        let (ok, _) = sh("zellij attach --create-background zf-one", &[]);
        assert!(ok);
        let pane_fn = format!("{ZELLIJ_FUNCTIONS}zj_pane \"$1\"");
        wait("a focused terminal pane", &|| {
            !sh(&pane_fn, &["zf-one"]).1.trim().is_empty()
        });
        let target = format!("terminal_{}", sh(&pane_fn, &["zf-one"]).1.trim());

        // type a line; the capture sees it, and the same screen answers SAME
        sh(
            &format!("zellij --session zf-one action write-chars --pane-id {target} 'echo marker-xyz-42'; zellij --session zf-one action send-keys --pane-id {target} Enter"),
            &[],
        );
        let script = zellij_capture_script();
        wait("the typed line shows in the capture", &|| {
            let (ok, out) = sh(&script, &["zf-one", ""]);
            ok && out.starts_with("SUM ") && out.contains("marker-xyz-42")
        });
        let (ok, first) = sh(&script, &["zf-one", ""]);
        assert!(ok && first.starts_with("SUM "), "{first}");
        let sum = first[4..first.find('\n').unwrap_or(first.len())].to_string();
        let (_, again) = sh(&script, &["zf-one", &sum]);
        assert_eq!(again.trim(), format!("SAME {sum}"));

        // an unknown session is an error and prints no other session name
        let (ok, out) = sh(&script, &["zf-none", ""]);
        assert!(!ok);
        assert!(!out.contains("zf-one"), "{out}");
        let (_, listed) = sh("zellij list-sessions --no-formatting", &[]);
        assert!(
            !listed.contains("zf-none"),
            "the call must not create a session"
        );

        // the agent in the focused pane is found by its environment
        if Command::new("node").arg("--version").output().is_ok() {
            let agent = dir.join("claude");
            std::fs::write(
                &agent,
                "#!/usr/bin/env node\nsetTimeout(() => {}, 60000);\n",
            )
            .expect("agent");
            std::fs::set_permissions(&agent, std::os::unix::fs::PermissionsExt::from_mode(0o700))
                .expect("chmod");
            sh(
                &format!("zellij --session zf-one action write-chars --pane-id {target} '{}'; zellij --session zf-one action send-keys --pane-id {target} Enter", agent.display()),
                &[],
            );
            let pane_script = format!(
                "{}{}",
                roots_prefix("printf 'PC_STATUS=no_session\\n';"),
                crate::pane_context::PANE_BODY_FOR_TESTS
            );
            wait("the fake agent is seen in the pane", &|| {
                let (_, out) = sh(&pane_script, &["zf-one", "zellij"]);
                let ctx = crate::pane_context::parse_pane_context(&out);
                ctx.kind == crate::pane_context::PaneKind::AiCli && ctx.agent == "claude"
            });
        } else {
            eprintln!("SKIP agent check: node is not installed");
        }
    }
}
