//! Differential test: the Rust host guard and the Python reference (`ssh-app-hook.py`) must pick
//! the same rule for thousands of generated tool calls, wrappers and quoting tricks included.

use serde_json::{json, Value};
use ssh_app_host::guard::guard_match;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

const DRIVER: &str = r#"
import importlib.util, json, sys
spec = importlib.util.spec_from_file_location("hook", sys.argv[1])
hook = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hook)
for line in sys.stdin:
    case = json.loads(line)
    print(json.dumps(hook.guard_match(case["rules"], case["tool"], case["input"], case["cwd"])), flush=True)
"#;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as usize
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.next() % items.len()]
    }
}

const PROGS: [&str; 8] = ["rm", "git", "curl", "ls", "RM", "/bin/rm", "chmod", "echo"];
const ARGS: [&str; 9] = [
    "-rf /",
    "push origin main",
    "x",
    "-n 5",
    "'a b'",
    "\"q\"",
    "$(rm y)",
    "`rm z`",
    "",
];
const WRAP: [&str; 10] = [
    "",
    "sudo ",
    "env A=1 ",
    "nice -n 5 ",
    "timeout 5 ",
    "command ",
    "xargs ",
    "nohup ",
    "time ",
    "sudo -u root ",
];
const JOIN: [&str; 6] = [" && ", "; ", " | ", " || ", "\n", " & "];
const RULES: [&str; 14] = [
    "Bash(rm:*)",
    "Bash(git push:*)",
    "Bash(curl*)",
    "Bash(ls)",
    "Bash(chmod 777 *)",
    "Bash",
    "Edit(**/.env)",
    "Edit(/work/secret.txt)",
    "Read(~/.ssh/**)",
    "Read(/etc/*)",
    "Read",
    "Write(*.pem)",
    "nonsense(",
    "Bash(echo:*)",
];
const TOOLS: [&str; 9] = [
    "Bash",
    "run_shell_command",
    "Edit",
    "write_file",
    "Read",
    "read_many_files",
    "view_file",
    "bash",
    "Other",
];
const PATHS: [&str; 8] = [
    "/work/secret.txt",
    "sub/../secret.txt",
    "/work/app/.env",
    "/etc/passwd",
    "/etc/ssh/x",
    "~/.ssh/id",
    "key.pem",
    "/tmp/a",
];

fn command(r: &mut Rng) -> String {
    let parts = 1 + r.next() % 3;
    let mut out = String::new();
    for i in 0..parts {
        if i > 0 {
            out.push_str(r.pick(&JOIN));
        }
        let wrapped = format!("{}{} {}", r.pick(&WRAP), r.pick(&PROGS), r.pick(&ARGS));
        match r.next() % 4 {
            0 => out.push_str(&format!("sh -c '{}'", wrapped.replace('\'', ""))),
            1 => out.push_str(&format!("bash -lc \"{}\"", wrapped.replace('"', ""))),
            _ => out.push_str(&wrapped),
        }
    }
    out
}

#[test]
fn rust_guard_agrees_with_the_python_reference() {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/agent-hooks/ssh-app-hook.py");
    let Ok(mut py) = Command::new("python3")
        .arg("-I")
        .arg("-c")
        .arg(DRIVER)
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    else {
        eprintln!("skipping: python3 missing");
        return;
    };
    let mut stdin = py.stdin.take().unwrap();
    let mut stdout = BufReader::new(py.stdout.take().unwrap());
    let mut rng = Rng(7);
    let mut mismatches = Vec::new();
    let mut hits = 0;
    for _ in 0..4000 {
        let tool = rng.pick(&TOOLS);
        let input = if tool.contains("ash") || tool == "run_shell_command" {
            json!({ "command": command(&mut rng) })
        } else if tool == "read_many_files" {
            json!({ "paths": [rng.pick(&PATHS), rng.pick(&PATHS)] })
        } else {
            json!({ "file_path": rng.pick(&PATHS) })
        };
        let rules: Vec<String> = (0..1 + rng.next() % 3)
            .map(|_| rng.pick(&RULES).to_string())
            .collect();
        let case = json!({"rules": rules, "tool": tool, "input": input, "cwd": "/work"});
        writeln!(stdin, "{case}").unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let python: Value = serde_json::from_str(line.trim()).unwrap();
        let rust = guard_match(&rules, tool, &input, "/work").map_or(Value::Null, Value::String);
        hits += usize::from(!python.is_null());
        if python != rust {
            mismatches.push(format!("{case} python={python} rust={rust}"));
        }
    }
    drop(stdin);
    let _ = py.wait();
    assert!(
        hits > 500,
        "the generator must exercise matching rules, got {hits}"
    );
    assert!(
        mismatches.is_empty(),
        "{} mismatches, first: {:?}",
        mismatches.len(),
        mismatches.first()
    );
}
