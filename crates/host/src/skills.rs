//! `ssh-app-host skills`: the project's agent skills as typed JSON (feature 53).
//!
//! One request object on stdin (`op`, `agent`, `project`, `name`, `content`), one reply object on
//! stdout. Only the project's own skills folder (`.claude/skills` for Claude Code, `.agents/skills`
//! for Antigravity) is ever written; user-level skills are listed read only. Every path is built
//! from a validated name under a canonical project root, and no link is followed. A deleted skill
//! moves to `.trash/<name>-<unix time>` (restorable for 30 days), a disabled one to `.disabled/`.
//! Nothing in a skill file is executed.

use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Largest `SKILL.md` the app edits (the whole request travels in one command line).
pub const MAX_SKILL_BYTES: usize = 100_000;
const MAX_LISTED: usize = 500;
const TRASH_DAYS: u64 = 30;

fn err(msg: &str) -> Value {
    json!({"ok": false, "error": msg})
}

pub fn valid_name(n: &str) -> bool {
    let b = n.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// The skills folder of `agent` under the canonical `project`; `.claude` / `skills` must be real
/// directories of the project, not links out of it.
fn skills_dir(project: &str, agent: &str, create: bool) -> Result<PathBuf, String> {
    let rel: [&str; 2] = match agent {
        "claude" => [".claude", "skills"],
        "antigravity" => [".agents", "skills"],
        _ => return Err("this agent has no skills".into()),
    };
    if !project.starts_with('/') {
        return Err("the project folder must be an absolute path".into());
    }
    let root =
        fs::canonicalize(project).map_err(|_| "the project folder does not exist".to_string())?;
    if !root.is_dir() {
        return Err("the project is not a folder".into());
    }
    let mut dir = root;
    for part in rel {
        dir.push(part);
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(format!("{part} is not a plain folder")),
            Err(_) if create => fs::create_dir(&dir).map_err(|e| e.to_string())?,
            Err(_) => {}
        }
    }
    Ok(dir)
}

fn plain_dir(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
}

fn plain_file(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.is_file())
}

/// `(name, description)` of a `SKILL.md` front matter; `None` when there is no front matter.
pub fn front_matter(text: &str) -> Option<(String, String)> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let end = rest.find("\n---")?;
    let (mut name, mut desc) = (String::new(), String::new());
    for line in rest[..end].lines() {
        if let Some(v) = line.strip_prefix("name:") {
            name = v.trim().trim_matches('"').trim_matches('\'').to_string();
        } else if let Some(v) = line.strip_prefix("description:") {
            desc = v.trim().trim_matches('"').trim_matches('\'').to_string();
        }
    }
    Some((name, desc))
}

fn check_content(name: &str, content: &str) -> Result<(), String> {
    if content.len() > MAX_SKILL_BYTES {
        return Err(format!("a skill is limited to {MAX_SKILL_BYTES} bytes"));
    }
    let Some((n, d)) = front_matter(content) else {
        return Err("the skill needs a front matter (--- name / description ---)".into());
    };
    if n != name {
        return Err("the front matter name must equal the skill's folder name".into());
    }
    if d.is_empty() {
        return Err("the description must not be empty".into());
    }
    Ok(())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn entry(dir: &Path, name: &str, scope: &str, enabled: bool, read_only: bool) -> Option<Value> {
    let file = dir.join(name).join("SKILL.md");
    if !plain_dir(&dir.join(name)) || !plain_file(&file) {
        return None;
    }
    let bytes = fs::metadata(&file).map_or(0, |m| m.len());
    let head = fs::read(&file).map(|mut b| {
        b.truncate(4096);
        String::from_utf8_lossy(&b).into_owned()
    });
    let (_, desc) = head.ok().and_then(|t| front_matter(&t)).unwrap_or_default();
    Some(json!({
        "name": name, "scope": scope, "description": desc.chars().take(300).collect::<String>(),
        "bytes": bytes, "tokens": bytes.div_ceil(4), "enabled": enabled, "read_only": read_only,
    }))
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| valid_name(n))
        .collect();
    v.sort();
    v.truncate(MAX_LISTED);
    v
}

fn list(project: &str, agent: &str, home: &str) -> Value {
    let dir = match skills_dir(project, agent, false) {
        Ok(d) => d,
        Err(e) => return err(&e),
    };
    let mut skills = vec![];
    for n in names(&dir) {
        skills.extend(entry(&dir, &n, "project", true, false));
    }
    let off = dir.join(".disabled");
    if plain_dir(&off) {
        for n in names(&off) {
            skills.extend(entry(&off, &n, "project", false, false));
        }
    }
    if agent == "claude" && home.starts_with('/') {
        let user = Path::new(home).join(".claude").join("skills");
        if plain_dir(&user) {
            for n in names(&user) {
                skills.extend(entry(&user, &n, "user", true, true));
            }
        }
    }
    json!({"ok": true, "skills": skills})
}

/// Folder of an enabled or disabled project skill.
fn locate(dir: &Path, name: &str) -> Option<(PathBuf, bool)> {
    let on = dir.join(name);
    if plain_dir(&on) {
        return Some((on, true));
    }
    let off = dir.join(".disabled").join(name);
    plain_dir(&off).then_some((off, false))
}

fn read(dir: &Path, name: &str, user_home: Option<&str>) -> Value {
    let folder = match user_home {
        Some(h) => {
            let d = Path::new(h).join(".claude").join("skills");
            plain_dir(&d.join(name)).then(|| d.join(name))
        }
        None => locate(dir, name).map(|(p, _)| p),
    };
    let Some(folder) = folder else {
        return err("no such skill");
    };
    let file = folder.join("SKILL.md");
    if !plain_file(&file) {
        return err("the skill has no SKILL.md");
    }
    let Ok(bytes) = fs::read(&file) else {
        return err("cannot read the skill");
    };
    if bytes.len() > 4 * MAX_SKILL_BYTES {
        return err("the skill is too large to open here");
    }
    let files: Vec<String> = fs::read_dir(&folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n != "SKILL.md" && !n.ends_with(".bak"))
        .take(100)
        .collect();
    json!({"ok": true, "content": String::from_utf8_lossy(&bytes), "files": files})
}

/// Writes `content` as `<folder>/SKILL.md` through a temp file; the previous text stays as `.bak`.
fn write_atomic(folder: &Path, content: &str) -> Result<(), String> {
    let target = folder.join("SKILL.md");
    let tmp = folder.join(format!(".SKILL.md.tmp{}", std::process::id()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    let done = f.write_all(content.as_bytes()).and_then(|()| f.sync_all());
    if let Err(e) = done {
        let _ = fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    if plain_file(&target) {
        let _ = fs::copy(&target, folder.join("SKILL.md.bak"));
    }
    fs::rename(&tmp, &target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

fn purge_trash(trash: &Path) {
    for e in fs::read_dir(trash).into_iter().flatten().flatten() {
        let p = e.path();
        let stamp = e
            .file_name()
            .to_str()
            .and_then(|n| n.rsplit_once('-'))
            .and_then(|(_, t)| t.parse::<u64>().ok());
        if let Some(t) = stamp {
            if plain_dir(&p) && now().saturating_sub(t) > TRASH_DAYS * 86_400 {
                let _ = fs::remove_dir_all(&p);
            }
        }
    }
}

/// Answers one request. `home` is only used to list and read user-level skills.
pub fn handle(req: &Value, home: &str) -> Value {
    let field = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("");
    let (op, agent, project, name) = (field("op"), field("agent"), field("project"), field("name"));
    if op == "list" {
        return list(project, agent, home);
    }
    if op == "trash" {
        return match skills_dir(project, agent, false) {
            Ok(dir) => {
                let trash = dir.join(".trash");
                json!({"ok": true, "trash": if plain_dir(&trash) { names_with_stamp(&trash) } else { vec![] }})
            }
            Err(e) => err(&e),
        };
    }
    if !valid_name(name) {
        return err("a skill name is lowercase letters, digits and hyphens (up to 64)");
    }
    let creating = op == "create";
    let dir = match skills_dir(project, agent, creating) {
        Ok(d) => d,
        Err(e) => return err(&e),
    };
    match op {
        "read" if field("scope") == "user" => read(&dir, name, Some(home)),
        "read" => read(&dir, name, None),
        "create" => {
            let content = field("content");
            if let Err(e) = check_content(name, content) {
                return err(&e);
            }
            if locate(&dir, name).is_some() {
                return err("a skill with this name already exists");
            }
            let folder = dir.join(name);
            if let Err(e) = fs::create_dir(&folder)
                .map_err(|e| e.to_string())
                .and_then(|()| write_atomic(&folder, content))
            {
                return err(&e);
            }
            json!({"ok": true})
        }
        "write" => {
            let content = field("content");
            if let Err(e) = check_content(name, content) {
                return err(&e);
            }
            match locate(&dir, name) {
                Some((folder, _)) => write_atomic(&folder, content)
                    .map_or_else(|e| err(&e), |()| json!({"ok": true})),
                None => err("no such skill"),
            }
        }
        "delete" => {
            let Some((folder, _)) = locate(&dir, name) else {
                return err("no such skill");
            };
            let trash = dir.join(".trash");
            if !plain_dir(&trash) && fs::create_dir(&trash).is_err() {
                return err("cannot create the trash folder");
            }
            purge_trash(&trash);
            let to = trash.join(format!("{name}-{}", now()));
            fs::rename(&folder, &to).map_or_else(
                |e| err(&e.to_string()),
                |()| json!({"ok": true, "trash": to.file_name().and_then(|n| n.to_str())}),
            )
        }
        "restore" => {
            // `name` is the trash entry (`<skill>-<unix time>`), validated like a name plus digits.
            let from = dir.join(".trash").join(name);
            let Some((skill, _)) = name.rsplit_once('-') else {
                return err("not a trash entry");
            };
            if !plain_dir(&from) || !valid_name(skill) {
                return err("no such trash entry");
            }
            if locate(&dir, skill).is_some() {
                return err("a skill with this name exists; rename or delete it first");
            }
            fs::rename(&from, dir.join(skill))
                .map_or_else(|e| err(&e.to_string()), |()| json!({"ok": true}))
        }
        "set_enabled" => {
            let enable = req.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            let Some((folder, enabled)) = locate(&dir, name) else {
                return err("no such skill");
            };
            if enabled == enable {
                return json!({"ok": true});
            }
            let to = if enable {
                dir.join(name)
            } else {
                dir.join(".disabled").join(name)
            };
            if !enable {
                let off = dir.join(".disabled");
                if !plain_dir(&off) && fs::create_dir(&off).is_err() {
                    return err("cannot create the folder for disabled skills");
                }
            }
            if to.exists() {
                return err("a skill with this name is already there");
            }
            fs::rename(&folder, &to).map_or_else(|e| err(&e.to_string()), |()| json!({"ok": true}))
        }
        _ => err("unknown operation"),
    }
}

fn names_with_stamp(trash: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(trash)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            n.rsplit_once('-')
                .is_some_and(|(s, t)| valid_name(s) && t.parse::<u64>().is_ok())
        })
        .collect();
    v.sort();
    v.truncate(MAX_LISTED);
    v
}
