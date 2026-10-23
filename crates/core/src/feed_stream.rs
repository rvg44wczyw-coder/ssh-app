//! The feed as a stream: one long-lived exec channel runs a small `sh` loop on the host that
//! watches the session file and the tail of the tmux pane and writes **events** only when
//! something changed. Nothing is polled by the phone; closing the channel (background, lost
//! connection) stops the loop.
//!
//! Wire format, one event per line of JSON, an optional body follows the line:
//! `{"ev":"hello","proto":1}`
//! `{"ev":"delta","len":N}\n<N bytes: the output of the cursor read (`feed_read`)>\n`
//! `{"ev":"live","sum":"123","len":N}\n<N bytes: the last lines of the pane>\n`

use crate::error::SshCoreError;
use crate::tmux::shell_quote;

/// Largest body the phone accepts in one event.
pub const MAX_BODY: usize = 8 * 1024 * 1024;
/// Lines of the pane sent as the live part.
pub const LIVE_LINES: u32 = 80;

/// `$1` session, `$2` transcript path, `$3` inode of the cursor or `-`, `$4` offset or `-`, `$5` quiet
/// ping period in loops (default 15). `$5` seconds between pings of a quiet stream (default 15).
/// `tmux` is the command word so a test can point it at a private socket.
pub fn stream_script(tmux: &str) -> String {
    stream_script_with(tmux, true)
}

/// `guard: false` only in tests (their temporary folders are under the blocked `/var`).
pub(crate) fn stream_script_with(tmux: &str, guard: bool) -> String {
    let delta = crate::feed_read::delta_script_with(guard).replace("exit 0", "return 0");
    format!(
        r#"PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"; export PATH
command -v tmux >/dev/null 2>&1 || {{ echo '{{"ev":"bye","why":"notmux"}}'; exit 0; }}
t=$(mktemp "${{TMPDIR:-/tmp}}/ssh-app-feed.XXXXXX") || exit 1
trap 'rm -f "$t"' EXIT HUP INT TERM
delta() {{
{delta}
}}
emit() {{
  n=$(wc -c < "$t" | tr -d ' ')
  printf '%s,"len":%s}}\n' "$1" "$n" || exit 0
  cat "$t" || exit 0
  printf '\n' || exit 0
}}
printf '{{"ev":"hello","proto":1}}\n' || exit 0
ino=$3; off=$4; lastsum=""; quiet=0; first=1
while :; do
  changed=0
  delta "$2" "$ino" "$off" > "$t" 2>/dev/null
  if [ "$(head -c 6 "$t")" != NOFILE ]; then
    cur=$(sed -n 3p "$t")
    nino=$(printf '%s' "$cur" | cut -d' ' -f2); noff=$(printf '%s' "$cur" | cut -d' ' -f3)
    if [ "$nino" != "$ino" ] || [ "$noff" != "$off" ] || [ "$(sed -n 2p "$t")" = "MODE full" ]; then
      if [ "$first" = 1 ] || [ "$nino" != "$ino" ] || [ "$noff" != "$off" ] || [ "$(sed -n 2p "$t")" = "MODE full" ]; then
        emit '{{"ev":"delta"'; ino=$nino; off=$noff; changed=1
      fi
    fi
  fi
  first=0
  {tmux} capture-pane -p -J -S -{live} -t "=$1:" > "$t" 2>/dev/null || {{ printf '{{"ev":"bye","why":"nopane"}}\n'; exit 0; }}
  sum=$(cksum < "$t" | cut -d' ' -f1)
  if [ "$sum" != "$lastsum" ]; then lastsum=$sum; emit "{{\"ev\":\"live\",\"sum\":\"$sum\""; changed=1; fi
  if [ "$changed" = 1 ]; then quiet=0; else quiet=$((quiet+1)); fi
  if [ "$quiet" -gt 30 ]; then
    # A quiet stream still writes now and then, so a reader that is gone is noticed.
    if [ $((quiet % ${{5:-15}})) = 0 ]; then printf '{{"ev":"ping"}}\n' || exit 0; fi
    sleep 1
  else sleep 0.3 2>/dev/null || sleep 1; fi
done"#,
        live = LIVE_LINES
    )
}

/// The command that starts the stream on the host (plain `sh`, arguments quoted; the caller wraps
/// it in `cancellable_sh` so that closing the channel stops it).
pub fn build_stream_command(
    session_name: &str,
    path: &str,
    cursor: Option<crate::feed_read::FeedCursor>,
) -> Result<String, SshCoreError> {
    crate::transcript::validate_transcript_path(path).map_err(|e| {
        SshCoreError::TranscriptError {
            reason: e.to_string(),
        }
    })?;
    let (ino, off) = match cursor {
        Some(c) => (c.inode.to_string(), c.offset.to_string()),
        None => ("-".to_string(), "-".to_string()),
    };
    Ok(format!(
        "exec sh -c {} sh {} {} {} {}",
        shell_quote(&stream_script("tmux")),
        shell_quote(session_name.trim()),
        shell_quote(path),
        ino,
        off
    ))
}

/// One event of the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Hello {
        proto: u32,
    },
    /// The output of a cursor read (parsed by `feed_read::parse_delta_output`).
    Delta(String),
    /// The last lines of the pane and their checksum.
    Live {
        sum: String,
        text: String,
    },
    Bye {
        why: String,
    },
}

/// Cuts the byte stream into events, whatever the chunking of the channel.
#[derive(Default)]
pub struct FrameParser {
    buf: Vec<u8>,
    /// A header was read and its body (`len` bytes and a newline) is awaited.
    waiting: Option<(String, Option<String>, usize)>,
}

impl FrameParser {
    pub fn push(&mut self, data: &[u8]) -> Result<Vec<StreamEvent>, SshCoreError> {
        self.buf.extend_from_slice(data);
        let bad = |why: &str| SshCoreError::IoError {
            reason: format!("Unexpected feed data: {why}"),
        };
        let mut out = Vec::new();
        loop {
            if let Some((ev, sum, len)) = self.waiting.clone() {
                if self.buf.len() < len + 1 {
                    break;
                }
                let body: Vec<u8> = self.buf.drain(..len).collect();
                if self.buf.remove(0) != b'\n' {
                    return Err(bad("body not closed"));
                }
                let text = String::from_utf8_lossy(&body).to_string();
                self.waiting = None;
                out.push(match (ev.as_str(), sum) {
                    ("delta", _) => StreamEvent::Delta(text),
                    ("live", Some(sum)) => StreamEvent::Live { sum, text },
                    _ => return Err(bad("unknown body")),
                });
                continue;
            }
            let Some(nl) = self.buf.iter().position(|b| *b == b'\n') else {
                if self.buf.len() > 4096 {
                    return Err(bad("header too long"));
                }
                break;
            };
            let line: Vec<u8> = self.buf.drain(..=nl).collect();
            let v: serde_json::Value =
                serde_json::from_slice(&line[..nl]).map_err(|_| bad("header is not JSON"))?;
            match v.get("ev").and_then(|e| e.as_str()) {
                Some("hello") => out.push(StreamEvent::Hello {
                    proto: v.get("proto").and_then(|p| p.as_u64()).unwrap_or(0) as u32,
                }),
                Some("bye") => out.push(StreamEvent::Bye {
                    why: v
                        .get("why")
                        .and_then(|w| w.as_str())
                        .unwrap_or("")
                        .chars()
                        .take(40)
                        .collect(),
                }),
                Some(ev @ ("delta" | "live")) => {
                    let len = v
                        .get("len")
                        .and_then(|l| l.as_u64())
                        .ok_or_else(|| bad("no length"))? as usize;
                    if len > MAX_BODY {
                        return Err(bad("body too large"));
                    }
                    let sum = v
                        .get("sum")
                        .and_then(|s| s.as_str())
                        .map(|s| s.chars().take(20).collect());
                    self.waiting = Some((ev.to_string(), sum, len));
                }
                // An event this app does not know is skipped (a newer host).
                _ => {}
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn frames_are_cut_the_same_way_at_every_chunking() {
        let mut wire = Vec::new();
        wire.extend_from_slice(b"{\"ev\":\"hello\",\"proto\":1}\n");
        let body = "TP_PATH=/a\nMODE full\nCUR 1 5\nПривет\n";
        wire.extend_from_slice(
            format!("{{\"ev\":\"delta\",\"len\":{}}}\n{}\n", body.len(), body).as_bytes(),
        );
        wire.extend_from_slice(b"{\"ev\":\"future\",\"x\":1}\n");
        wire.extend_from_slice(b"{\"ev\":\"live\",\"sum\":\"77\",\"len\":3}\nabc\n{\"ev\":\"bye\",\"why\":\"nopane\"}\n");
        let want = vec![
            StreamEvent::Hello { proto: 1 },
            StreamEvent::Delta(body.to_string()),
            StreamEvent::Live {
                sum: "77".into(),
                text: "abc".into(),
            },
            StreamEvent::Bye {
                why: "nopane".into(),
            },
        ];
        for size in [1usize, 2, 3, 7, 13, 64, wire.len()] {
            let mut p = FrameParser::default();
            let mut got = Vec::new();
            for c in wire.chunks(size) {
                got.extend(p.push(c).expect("frames"));
            }
            assert_eq!(got, want, "chunk size {size}");
        }
    }

    #[test]
    fn hostile_streams_are_refused() {
        assert!(FrameParser::default().push(b"not json\n").is_err());
        assert!(FrameParser::default().push(&vec![b'a'; 5000]).is_err());
        assert!(FrameParser::default()
            .push(b"{\"ev\":\"delta\",\"len\":99999999999}\n")
            .is_err());
        assert!(FrameParser::default()
            .push(b"{\"ev\":\"delta\"}\n")
            .is_err());
        let mut p = FrameParser::default();
        assert!(
            p.push(b"{\"ev\":\"live\",\"sum\":\"1\",\"len\":1}\nXY")
                .is_err(),
            "body must end with a newline"
        );
    }

    #[test]
    fn the_command_refuses_bad_paths_and_quotes_the_rest() {
        assert!(build_stream_command("s", "../x", None).is_err());
        let c = build_stream_command(
            "my session",
            "/home/u/a.jsonl",
            Some(crate::feed_read::FeedCursor {
                inode: 3,
                offset: 4,
            }),
        )
        .expect("ok");
        assert!(c.contains("'my session' '/home/u/a.jsonl' 3 4"), "{c}");
    }

    struct Private {
        dir: tempfile::TempDir,
    }
    impl Private {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("dir");
            std::fs::set_permissions(
                dir.path(),
                std::os::unix::fs::PermissionsExt::from_mode(0o700),
            )
            .expect("chmod");
            Private { dir }
        }
        fn sock(&self) -> String {
            self.dir.path().join("sock").to_string_lossy().to_string()
        }
        fn tmux(&self, args: &[&str]) {
            Command::new("tmux")
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .args(["-S", &self.sock(), "-f", "/dev/null"])
                .args(args)
                .output()
                .expect("tmux");
        }
    }
    impl Drop for Private {
        fn drop(&mut self) {
            self.tmux(&["kill-server"]);
        }
    }

    /// Runs the stream script for real and delivers its events through a channel.
    fn start(
        p: &Private,
        file: &std::path::Path,
        cursor: &str,
    ) -> (std::process::Child, mpsc::Receiver<StreamEvent>) {
        let script = stream_script_with(&format!("tmux -S '{}'", p.sock()), false);
        let mut child = Command::new("sh")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .args([
                "-c",
                &script,
                "sh",
                "cap",
                &file.to_string_lossy(),
                cursor.split(' ').next().unwrap_or("-"),
                cursor.split(' ').nth(1).unwrap_or("-"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .expect("sh");
        let mut out = child.stdout.take().expect("stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut parser = FrameParser::default();
            let mut buf = [0u8; 4096];
            while let Ok(n) = out.read(&mut buf) {
                if n == 0 {
                    break;
                }
                for ev in parser.push(&buf[..n]).unwrap_or_default() {
                    if tx.send(ev).is_err() {
                        return;
                    }
                }
            }
        });
        (child, rx)
    }

    fn wait_for(
        rx: &mpsc::Receiver<StreamEvent>,
        what: &str,
        f: impl Fn(&StreamEvent) -> bool,
    ) -> StreamEvent {
        let end = Instant::now() + Duration::from_secs(8);
        while Instant::now() < end {
            if let Ok(ev) = rx.recv_timeout(Duration::from_millis(200)) {
                if f(&ev) {
                    return ev;
                }
            }
        }
        panic!("no event: {what}");
    }

    fn user(n: usize) -> String {
        format!("{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"m{n}\"}}}}\n")
    }

    #[test]
    fn the_stream_sends_the_file_and_the_pane_as_events_and_ends_when_the_reader_goes() {
        let p = Private::new();
        p.tmux(&[
            "new-session",
            "-d",
            "-s",
            "cap",
            "-x",
            "100",
            "-y",
            "20",
            "sh",
        ]);
        let file = p.dir.path().join("s.jsonl");
        std::fs::write(&file, format!("{}{}", user(1), user(2))).expect("w");
        let (mut child, rx) = start(&p, &file, "- -");

        assert!(matches!(
            wait_for(&rx, "hello", |e| matches!(e, StreamEvent::Hello { .. })),
            StreamEvent::Hello { proto: 1 }
        ));
        let state = crate::feed::FeedState::new();
        let StreamEvent::Delta(first) =
            wait_for(&rx, "first delta", |e| matches!(e, StreamEvent::Delta(_)))
        else {
            unreachable!()
        };
        let read = crate::feed_read::parse_inner(&first, false)
            .expect("parse")
            .expect("file");
        assert!(read.reset);
        state.reset();
        state.apply_chunk(read.text);
        assert_eq!(state.turns().len(), 2);

        // The pane: a live event with its tail.
        p.tmux(&["send-keys", "-t", "cap", "echo live-marker", "Enter"]);
        let StreamEvent::Live { text, .. } = wait_for(
            &rx,
            "live",
            |e| matches!(e, StreamEvent::Live { text, .. } if text.contains("live-marker")),
        ) else {
            unreachable!()
        };
        assert!(text.contains("live-marker"));

        // Appending to the file: one more delta with only the new lines, no reset.
        let mut h = std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .expect("open");
        h.write_all(user(3).as_bytes()).expect("w");
        let StreamEvent::Delta(d) =
            wait_for(&rx, "second delta", |e| matches!(e, StreamEvent::Delta(_)))
        else {
            unreachable!()
        };
        let read = crate::feed_read::parse_inner(&d, false)
            .expect("parse")
            .expect("file");
        assert!(!read.reset);
        assert_eq!(read.text, user(3));
        state.apply_chunk(read.text);
        assert_eq!(state.turns().len(), 3);

        // Nothing changes: no event arrives for a while (the host stays quiet).
        assert!(rx.recv_timeout(Duration::from_millis(1200)).is_err());

        // The file is replaced: a reset arrives.
        let other = p.dir.path().join("other.jsonl");
        std::fs::write(&other, user(9)).expect("w");
        std::fs::rename(&other, &file).expect("rename");
        let StreamEvent::Delta(d) =
            wait_for(&rx, "reset delta", |e| matches!(e, StreamEvent::Delta(_)))
        else {
            unreachable!()
        };
        assert!(
            crate::feed_read::parse_inner(&d, false)
                .expect("parse")
                .expect("file")
                .reset
        );

        // The reader goes away: the script ends by itself (a write fails or the pipe is gone).
        drop(rx);
        let end = Instant::now() + Duration::from_secs(8);
        // The first change makes the reader thread notice that nobody listens and close the pipe;
        // the next change makes the script's write fail, and it ends.
        for word in ["echo more", "echo more2", "echo more3"] {
            p.tmux(&["send-keys", "-t", "cap", word, "Enter"]);
            std::thread::sleep(Duration::from_millis(900));
        }
        loop {
            if child.try_wait().expect("wait").is_some() {
                break;
            }
            assert!(Instant::now() < end, "the stream script did not end");
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.stdout;
    }

    #[test]
    fn a_vanished_pane_ends_the_stream_with_bye() {
        let p = Private::new();
        p.tmux(&["new-session", "-d", "-s", "other", "sh"]);
        let file = p.dir.path().join("s.jsonl");
        std::fs::write(&file, user(1)).expect("w");
        let (mut child, rx) = start(&p, &file, "- -");
        wait_for(
            &rx,
            "bye",
            |e| matches!(e, StreamEvent::Bye { why } if why == "nopane"),
        );
        let _ = child.wait();
        let _ = std::io::stdout().flush();
        let _ = std::io::BufReader::new(std::io::empty()).lines().count();
    }

    #[test]
    fn a_hostile_session_name_or_path_is_only_data() {
        let p = Private::new();
        p.tmux(&["new-session", "-d", "-s", "cap", "sh"]);
        let marker = p.dir.path().join("pwned");
        let file = p.dir.path().join("a b;touch pwned.jsonl");
        std::fs::write(&file, user(1)).expect("w");
        let script = stream_script_with(&format!("tmux -S '{}'", p.sock()), false);
        for (name, path) in [
            (
                format!("x; touch {}", marker.display()),
                file.to_string_lossy().to_string(),
            ),
            (
                "cap".to_string(),
                format!("{}$(touch {})", file.display(), marker.display()),
            ),
            ("cap".to_string(), format!("`touch {}`", marker.display())),
        ] {
            let out = Command::new("timeout")
                .args(["2", "sh", "-c", &script, "sh", &name, &path, "-", "-"])
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .output();
            // `timeout` may be absent on a Mac; then the plain script is run with a short life.
            if out.is_err() {
                let mut child = Command::new("sh")
                    .args(["-c", &script, "sh", &name, &path, "-", "-"])
                    .env_remove("TMUX")
                    .env_remove("TMUX_PANE")
                    .stdout(Stdio::null())
                    .spawn()
                    .expect("sh");
                std::thread::sleep(Duration::from_millis(1200));
                let _ = child.kill();
                let _ = child.wait();
            }
            assert!(!marker.exists(), "{name} / {path} ran a command");
        }
    }
}
