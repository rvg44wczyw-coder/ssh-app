//! Listening TCP ports of the host (the "Ports" screen): `ss -ltnp` on Linux, `lsof` on macOS.
//! The output of the host is untrusted display text: it is parsed strictly, stripped of control
//! characters and capped.

const MAX_PORTS: usize = 500;

/// The command run on the host; read-only, no shell input from the app.
pub const LISTENING_PORTS_COMMAND: &str = "if command -v ss >/dev/null 2>&1; then ss -ltnpH 2>/dev/null; else lsof -nP -iTCP -sTCP:LISTEN 2>/dev/null; fi; true";

/// One listening port (IPv4 and IPv6 sockets of the same program are merged).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ListeningPort {
    pub port: u16,
    /// Program name; empty when the host did not say (other accounts' sockets).
    pub process: String,
    pub pid: Option<u32>,
    /// Only reachable from the host itself (127.0.0.1 / ::1): a dev server, not an open service.
    pub local_only: bool,
}

/// Reads the output of [`LISTENING_PORTS_COMMAND`], sorted by port.
pub fn parse_listening_ports(text: &str) -> Vec<ListeningPort> {
    let mut out: Vec<ListeningPort> = Vec::new();
    for line in text.lines() {
        let Some((addr, process, pid)) = parse_ss(line).or_else(|| parse_lsof(line)) else {
            continue;
        };
        let Some((host, port)) = addr.rsplit_once(':') else {
            continue;
        };
        let Ok(port) = port.parse::<u16>() else {
            continue;
        };
        let local = host == "127.0.0.1" || host == "[::1]" || host.starts_with("127.");
        if let Some(p) = out
            .iter_mut()
            .find(|p| p.port == port && p.process == process)
        {
            p.local_only &= local;
            if p.pid.is_none() {
                p.pid = pid;
            }
        } else if out.len() < MAX_PORTS {
            out.push(ListeningPort {
                port,
                process,
                pid,
                local_only: local,
            });
        }
    }
    out.sort_by_key(|p| (p.port, p.process.clone()));
    out
}

fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(60).collect()
}

/// `LISTEN 0 128 0.0.0.0:22 0.0.0.0:* users:(("sshd",pid=812,fd=3))`
fn parse_ss(line: &str) -> Option<(String, String, Option<u32>)> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.first() != Some(&"LISTEN") || f.len() < 5 {
        return None;
    }
    let (mut process, mut pid) = (String::new(), None);
    if let Some(users) = f
        .get(5..)
        .and_then(|r| r.iter().find(|x| x.starts_with("users:")))
    {
        if let Some(rest) = users.split("((\"").nth(1) {
            process = clean(rest.split('"').next().unwrap_or(""));
            pid = rest
                .split("pid=")
                .nth(1)
                .and_then(|p| p.split(',').next())
                .and_then(|p| p.parse().ok());
        }
    }
    Some((f[3].to_string(), process, pid))
}

/// `node 4242 dev 23u IPv4 0x1 0t0 TCP 127.0.0.1:3000 (LISTEN)`
fn parse_lsof(line: &str) -> Option<(String, String, Option<u32>)> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.len() < 9 || f.first() == Some(&"COMMAND") || f.last() != Some(&"(LISTEN)") {
        return None;
    }
    let pid = f[1].parse().ok();
    let name = f[f.len() - 2];
    let name = if let Some(rest) = name.strip_prefix('*') {
        format!("0.0.0.0{rest}")
    } else {
        name.to_string()
    };
    Some((name, clean(&f[0].replace("\\x20", " ")), pid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_ss_output_is_read_and_merged() {
        let t = "LISTEN 0 128 0.0.0.0:22 0.0.0.0:* users:((\"sshd\",pid=812,fd=3))\n\
                 LISTEN 0 128 [::]:22 [::]:* users:((\"sshd\",pid=812,fd=4))\n\
                 LISTEN 0 511 127.0.0.1:3000 0.0.0.0:* users:((\"node\",pid=4242,fd=19))\n\
                 LISTEN 0 4096 127.0.0.53%lo:53 0.0.0.0:*\n";
        let p = parse_listening_ports(t);
        assert_eq!(p.len(), 3);
        assert_eq!(
            (p[0].port, p[0].process.as_str(), p[0].local_only),
            (22, "sshd", false)
        );
        assert_eq!(p[1].port, 53);
        assert!(p[1].process.is_empty());
        assert_eq!(
            (p[2].port, p[2].pid, p[2].local_only),
            (3000, Some(4242), true)
        );
    }

    #[test]
    fn macos_lsof_output_is_read() {
        let t = "COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\n\
                 rapportd 512 dev 8u IPv4 0xabc 0t0 TCP *:49152 (LISTEN)\n\
                 node 4242 dev 23u IPv6 0xdef 0t0 TCP [::1]:5173 (LISTEN)\n\
                 Google\\x20Chrome 90 dev 5u IPv4 0x1 0t0 TCP 127.0.0.1:9222 (LISTEN)\n\
                 ssh 7 dev 3u IPv4 0x2 0t0 TCP 1.2.3.4:5555->5.6.7.8:22 (ESTABLISHED)\n";
        let p = parse_listening_ports(t);
        assert_eq!(
            p.iter().map(|x| x.port).collect::<Vec<_>>(),
            vec![5173, 9222, 49152]
        );
        assert!(p[0].local_only && p[1].local_only && !p[2].local_only);
        assert_eq!(p[1].process, "Google Chrome");
    }

    #[test]
    fn garbage_control_characters_and_floods_are_bounded() {
        assert!(
            parse_listening_ports("hello\nLISTEN\n\u{1b}[31mLISTEN 0 0 a:99999 b\n").is_empty()
        );
        let t = "LISTEN 0 1 0.0.0.0:80 x users:((\"a\u{7}b\",pid=1,fd=1))";
        assert_eq!(parse_listening_ports(t)[0].process, "ab");
        let mut many = String::new();
        for i in 1..=2000u32 {
            many.push_str(&format!("LISTEN 0 1 0.0.0.0:{i} x\n"));
        }
        assert_eq!(parse_listening_ports(&many).len(), 500);
    }
}
