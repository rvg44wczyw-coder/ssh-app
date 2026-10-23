//! The tmux pane capture of the feed, done in one host script: the host waits until the screen
//! stands still (the agent redraws its last lines while it writes), compares a checksum with the
//! one the app already has, and sends the text compressed. The app used to take up to four
//! captures 350 ms apart and move the whole history uncompressed each time.
//!
//! The script runs under plain `sh` (no login shell: it only needs `PATH`, which it sets); a host
//! where tmux is not found that way is asked again through the login shell.

use crate::error::SshCoreError;
use base64::Engine;
use std::io::Read;

/// Largest text accepted after decompression (the same bound as the old capture).
pub const MAX_CAPTURE_CHARS: usize = 3_000_000;
/// Compressed answers above this are refused before decoding.
const MAX_COMPRESSED: usize = 8 * 1024 * 1024;

/// What the script said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureAnswer {
    /// tmux was not found on the `PATH` of a plain `sh`.
    NoTmux,
    /// The same checksum as the app has.
    Same(String),
    /// New text with its checksum.
    Text { checksum: String, text: String },
}

/// `$1` session name, `$2` known checksum (digits or empty). `tmux` is the command word, so a
/// test can point it at a private socket.
pub fn capture_script(tmux: &str) -> String {
    format!(
        r#"PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"; export PATH
command -v tmux >/dev/null 2>&1 || {{ echo NOTMUX; exit 0; }}
cap() {{ {tmux} capture-pane -e -p -J -S - -t "=$1:" 2>/dev/null; }}
out=$(cap "$1") || exit 1
sum=$(printf '%s' "$out" | cksum | cut -d' ' -f1)
if [ "$sum" = "$2" ]; then echo "SAME $sum"; exit 0; fi
n=0
while [ "$n" -lt 3 ]; do
  sleep 0.15 2>/dev/null || sleep 1
  again=$(cap "$1") || break
  [ "$again" = "$out" ] && break
  out=$again; n=$((n+1))
done
sum=$(printf '%s' "$out" | cksum | cut -d' ' -f1)
if [ "$sum" = "$2" ]; then echo "SAME $sum"; exit 0; fi
if [ "${{#out}}" -gt 2000 ] && command -v gzip >/dev/null 2>&1 && command -v base64 >/dev/null 2>&1; then
  echo "GZ $sum"; printf '%s\n' "$out" | gzip -c | base64 | tr -d '\n'; echo
else
  echo "RAW $sum"; printf '%s\n' "$out"
fi
exit 0"#
    )
}

/// Reads the script's output.
pub fn parse_capture_output(output: &str) -> Result<CaptureAnswer, SshCoreError> {
    let bad = |reason: &str| SshCoreError::IoError {
        reason: reason.to_string(),
    };
    let (first, rest) = output.split_once('\n').unwrap_or((output, ""));
    let first = first.trim_end();
    if first == "NOTMUX" {
        return Ok(CaptureAnswer::NoTmux);
    }
    if let Some(sum) = first.strip_prefix("SAME ") {
        return Ok(CaptureAnswer::Same(digits(sum)?));
    }
    // The old answer format (`SUM`) is still understood.
    if let Some(sum) = first
        .strip_prefix("SUM ")
        .or_else(|| first.strip_prefix("RAW "))
    {
        let text: String = rest.chars().take(MAX_CAPTURE_CHARS).collect();
        return Ok(CaptureAnswer::Text {
            checksum: digits(sum)?,
            text,
        });
    }
    if let Some(sum) = first.strip_prefix("GZ ") {
        let checksum = digits(sum)?;
        let b64 = rest.trim();
        if b64.len() > MAX_COMPRESSED {
            return Err(bad("The screen is too large"));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|_| bad("The screen could not be decoded"))?;
        // Bounded inflate: a hostile or broken host cannot make the phone allocate without limit.
        let mut text = String::new();
        flate2::read::GzDecoder::new(&bytes[..])
            .take((MAX_CAPTURE_CHARS * 4) as u64)
            .read_to_string(&mut text)
            .map_err(|_| bad("The screen could not be decompressed"))?;
        let text: String = text.chars().take(MAX_CAPTURE_CHARS).collect();
        return Ok(CaptureAnswer::Text { checksum, text });
    }
    Err(bad("The pane could not be read"))
}

fn digits(s: &str) -> Result<String, SshCoreError> {
    let s = s.trim();
    if s.is_empty() || s.len() > 20 || !s.chars().all(|c| c.is_ascii_digit()) {
        return Err(SshCoreError::IoError {
            reason: "The pane could not be read".to_string(),
        });
    }
    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

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
        fn tmux(&self, args: &[&str]) -> std::process::Output {
            Command::new("tmux")
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .args(["-S", &self.sock(), "-f", "/dev/null"])
                .args(args)
                .output()
                .expect("tmux")
        }
        fn run(&self, name: &str, known: &str) -> String {
            let script = capture_script(&format!("tmux -S '{}'", self.sock()));
            let out = Command::new("sh")
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .args(["-c", &script, "sh", name, known])
                .output()
                .expect("sh");
            String::from_utf8_lossy(&out.stdout).to_string()
        }
    }

    impl Drop for Private {
        fn drop(&mut self) {
            // Only the private server, by its own socket.
            let _ = self.tmux(&["kill-server"]);
        }
    }

    #[test]
    fn the_script_returns_same_for_a_known_checksum_and_compressed_text_otherwise() {
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
        let big = "line of an answer with some words, repeated\n".repeat(200);
        let f = p.dir.path().join("big.txt");
        std::fs::write(&f, &big).expect("w");
        p.tmux(&[
            "send-keys",
            "-t",
            "cap",
            &format!("cat '{}'", f.display()),
            "Enter",
        ]);
        std::thread::sleep(std::time::Duration::from_millis(600));

        let first = parse_capture_output(&p.run("cap", "")).expect("parse");
        let CaptureAnswer::Text { checksum, text } = first else {
            panic!("{first:?}")
        };
        assert!(text.contains("line of an answer with some words"));
        assert!(text.lines().count() > 150, "the whole history is returned");
        let raw = p.run("cap", "");
        assert!(raw.starts_with("GZ "), "big output is compressed");
        assert!(
            raw.len() < big.len() / 3,
            "compressed answer is much smaller: {} vs {}",
            raw.len(),
            big.len()
        );

        let again = parse_capture_output(&p.run("cap", &checksum)).expect("parse");
        assert_eq!(again, CaptureAnswer::Same(checksum));
    }

    #[test]
    fn the_script_waits_for_a_stable_frame_and_small_screens_are_not_compressed() {
        let p = Private::new();
        p.tmux(&[
            "new-session",
            "-d",
            "-s",
            "cap",
            "-x",
            "80",
            "-y",
            "10",
            "sh",
        ]);
        p.tmux(&["send-keys", "-t", "cap", "echo hello", "Enter"]);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let out = p.run("cap", "");
        assert!(out.starts_with("RAW "), "{out}");
        let CaptureAnswer::Text { text, .. } = parse_capture_output(&out).expect("parse") else {
            panic!()
        };
        assert!(text.contains("hello"));

        // A pane that keeps changing: the script returns after at most three more looks.
        p.tmux(&[
            "send-keys",
            "-t",
            "cap",
            "while :; do date +%N; sleep 0.05; done",
            "Enter",
        ]);
        std::thread::sleep(std::time::Duration::from_millis(200));
        let t = std::time::Instant::now();
        let out = p.run("cap", "");
        assert!(
            t.elapsed() < std::time::Duration::from_secs(3),
            "{:?}",
            t.elapsed()
        );
        assert!(parse_capture_output(&out).is_ok());
    }

    #[test]
    fn a_missing_session_fails_and_a_hostile_name_is_only_an_argument() {
        let p = Private::new();
        p.tmux(&["new-session", "-d", "-s", "cap", "sh"]);
        assert!(parse_capture_output(&p.run("nope", "")).is_err());
        let marker = p.dir.path().join("pwned");
        let name = format!("x; touch {}", marker.display());
        let _ = p.run(&name, "");
        assert!(!marker.exists());
    }

    #[test]
    fn the_server_of_the_user_is_not_the_one_used() {
        // The private socket is a different server from the one in $TMUX.
        let p = Private::new();
        p.tmux(&["new-session", "-d", "-s", "cap", "sh"]);
        let theirs = Command::new("tmux")
            .env_remove("TMUX")
            .args(["list-sessions", "-F", "#{session_name}"])
            .output()
            .ok();
        let theirs = theirs
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        assert!(
            !theirs.lines().any(|l| l == "cap"),
            "the test session must exist only on the private socket"
        );
    }

    #[test]
    fn answers_are_parsed_strictly() {
        assert_eq!(
            parse_capture_output("NOTMUX\n").expect("ok"),
            CaptureAnswer::NoTmux
        );
        assert_eq!(
            parse_capture_output("SAME 123\n").expect("ok"),
            CaptureAnswer::Same("123".into())
        );
        assert!(parse_capture_output("SAME x1\n").is_err());
        assert!(parse_capture_output("GZ 1\n!!!notbase64!!!\n").is_err());
        assert!(
            parse_capture_output("GZ 1\naGVsbG8=\n").is_err(),
            "not gzip"
        );
        assert!(parse_capture_output("whatever\n").is_err());
        // The old format is still read.
        assert_eq!(
            parse_capture_output("SUM 5\nabc\n").expect("ok"),
            CaptureAnswer::Text {
                checksum: "5".into(),
                text: "abc\n".into()
            }
        );
        // A gzip bomb is cut at the bound.
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        std::io::Write::write_all(&mut enc, &vec![b'a'; 40_000_000]).expect("w");
        let bomb = base64::engine::general_purpose::STANDARD.encode(enc.finish().expect("gz"));
        let CaptureAnswer::Text { text, .. } =
            parse_capture_output(&format!("GZ 1\n{bomb}\n")).expect("bounded")
        else {
            panic!()
        };
        assert!(text.len() <= MAX_CAPTURE_CHARS);
    }
}
