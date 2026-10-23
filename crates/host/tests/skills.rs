//! The skills manager (feature 53) in temp folders only: never the real `~/.claude` or `~/.agents`.

use serde_json::{json, Value};
use ssh_app_host::skills::{handle, MAX_SKILL_BYTES};
use std::fs;
use std::os::unix::fs::symlink;
use tempfile::TempDir;

const SKILL: &str = "---\nname: review\ndescription: Reviews a diff\n---\n\nBody.\n";

struct Env {
    project: TempDir,
    home: TempDir,
}

impl Env {
    fn new() -> Env {
        Env {
            project: tempfile::tempdir().unwrap(),
            home: tempfile::tempdir().unwrap(),
        }
    }
    fn call(&self, mut req: Value) -> Value {
        req["project"] = json!(self.project.path().to_str().unwrap());
        req["agent"] = req.get("agent").cloned().unwrap_or(json!("claude"));
        handle(&req, self.home.path().to_str().unwrap())
    }
    fn skills(&self) -> std::path::PathBuf {
        fs::canonicalize(self.project.path())
            .unwrap()
            .join(".claude/skills")
    }
}

fn ok(v: &Value) -> bool {
    v["ok"] == true
}

#[test]
fn create_list_read_write_delete_restore() {
    let e = Env::new();
    assert!(ok(
        &e.call(json!({"op":"create","name":"review","content":SKILL}))
    ));
    let l = e.call(json!({"op":"list"}));
    assert_eq!(l["skills"][0]["name"], "review");
    assert_eq!(l["skills"][0]["description"], "Reviews a diff");
    assert_eq!(l["skills"][0]["scope"], "project");
    assert_eq!(l["skills"][0]["tokens"], SKILL.len().div_ceil(4));
    let r = e.call(json!({"op":"read","name":"review"}));
    assert_eq!(r["content"], SKILL);

    let edited = SKILL.replace("Body.", "Changed.");
    assert!(ok(
        &e.call(json!({"op":"write","name":"review","content":edited}))
    ));
    assert_eq!(
        fs::read_to_string(e.skills().join("review/SKILL.md")).unwrap(),
        edited
    );
    assert_eq!(
        fs::read_to_string(e.skills().join("review/SKILL.md.bak")).unwrap(),
        SKILL
    );

    let d = e.call(json!({"op":"delete","name":"review"}));
    assert!(ok(&d));
    assert!(!e.skills().join("review").exists());
    let entry = d["trash"].as_str().unwrap().to_string();
    assert_eq!(e.call(json!({"op":"trash"}))["trash"][0], entry);
    assert!(ok(&e.call(json!({"op":"restore","name":entry}))));
    assert!(e.skills().join("review/SKILL.md").is_file());
}

#[test]
fn disabling_moves_the_skill_aside_and_back() {
    let e = Env::new();
    e.call(json!({"op":"create","name":"review","content":SKILL}));
    assert!(ok(&e.call(
        json!({"op":"set_enabled","name":"review","enabled":false})
    )));
    assert!(e.skills().join(".disabled/review/SKILL.md").is_file());
    let l = e.call(json!({"op":"list"}));
    assert_eq!(l["skills"][0]["enabled"], false);
    assert!(ok(&e.call(
        json!({"op":"set_enabled","name":"review","enabled":true})
    )));
    assert!(e.skills().join("review/SKILL.md").is_file());
}

#[test]
fn bad_names_and_content_are_refused() {
    let e = Env::new();
    for name in [
        "",
        "../x",
        "A",
        "a/b",
        "a b",
        "-x",
        ".trash",
        &"a".repeat(65),
    ] {
        let v = e.call(json!({"op":"create","name":name,"content":SKILL}));
        assert!(!ok(&v), "{name}");
    }
    let bad = [
        "no front matter",
        "---\nname: other\ndescription: x\n---\n",
        "---\nname: review\ndescription:\n---\n",
        &format!(
            "---\nname: review\ndescription: x\n---\n{}",
            "a".repeat(MAX_SKILL_BYTES)
        ),
    ];
    for c in bad {
        assert!(!ok(
            &e.call(json!({"op":"create","name":"review","content":c}))
        ));
    }
    assert!(!e.skills().join("review").exists());
    e.call(json!({"op":"create","name":"review","content":SKILL}));
    assert!(
        !ok(&e.call(json!({"op":"create","name":"review","content":SKILL}))),
        "exists"
    );
    assert!(!ok(
        &e.call(json!({"op":"write","name":"missing","content":SKILL}))
    ));
    assert!(!ok(&e.call(json!({"op":"restore","name":"review"}))));
}

#[test]
fn links_out_of_the_project_are_never_followed() {
    let e = Env::new();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(outside.path().join("skills")).unwrap();
    symlink(outside.path(), e.project.path().join(".claude")).unwrap();
    let v = e.call(json!({"op":"create","name":"review","content":SKILL}));
    assert!(!ok(&v), "{v}");
    assert!(!outside.path().join("skills/review").exists());

    // A skill folder that is a link, and a SKILL.md that is a link, are not listed or read.
    let e = Env::new();
    e.call(json!({"op":"create","name":"real","content":SKILL.replace("review", "real")}));
    symlink(outside.path(), e.skills().join("linked")).unwrap();
    fs::create_dir(e.skills().join("filelink")).unwrap();
    fs::write(outside.path().join("secret"), "x").unwrap();
    symlink(
        outside.path().join("secret"),
        e.skills().join("filelink/SKILL.md"),
    )
    .unwrap();
    let l = e.call(json!({"op":"list"}));
    let names: Vec<_> = l["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["real"]);
    assert!(!ok(&e.call(json!({"op":"read","name":"filelink"}))));
}

#[test]
fn user_skills_are_listed_read_only_and_cannot_be_written() {
    let e = Env::new();
    let user = e.home.path().join(".claude/skills/mine");
    fs::create_dir_all(&user).unwrap();
    fs::write(user.join("SKILL.md"), SKILL.replace("review", "mine")).unwrap();
    let l = e.call(json!({"op":"list"}));
    assert_eq!(l["skills"][0]["scope"], "user");
    assert_eq!(l["skills"][0]["read_only"], true);
    assert!(ok(
        &e.call(json!({"op":"read","name":"mine","scope":"user"}))
    ));
    assert!(!ok(&e.call(
        json!({"op":"write","name":"mine","content":SKILL.replace("review", "mine")})
    )));
    assert!(!ok(&e.call(json!({"op":"delete","name":"mine"}))));
    assert!(user.join("SKILL.md").is_file());
}

#[test]
fn antigravity_uses_its_own_folder_and_ollama_has_none() {
    let e = Env::new();
    assert!(ok(&e.call(
        json!({"agent":"antigravity","op":"create","name":"review","content":SKILL})
    )));
    assert!(fs::canonicalize(e.project.path())
        .unwrap()
        .join(".agents/skills/review/SKILL.md")
        .is_file());
    assert!(!ok(&e.call(json!({"agent":"ollama","op":"list"}))));
}

#[test]
fn old_trash_is_purged_and_relative_project_refused() {
    let e = Env::new();
    e.call(json!({"op":"create","name":"review","content":SKILL}));
    let old = e.skills().join(".trash/old-1000");
    fs::create_dir_all(&old).unwrap();
    e.call(json!({"op":"delete","name":"review"}));
    assert!(!old.exists(), "a month-old entry is removed");
    let v = handle(
        &json!({"op":"list","agent":"claude","project":"relative/dir"}),
        "/nonexistent",
    );
    assert!(!ok(&v));
}
