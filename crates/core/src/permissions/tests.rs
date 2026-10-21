use super::*;

fn p(allow: &[&str], ask: &[&str], deny: &[&str], mode: Option<&str>) -> AgentPermissions {
    AgentPermissions {
        allow: strings(allow),
        ask: strings(ask),
        deny: strings(deny),
        default_mode: mode.map(str::to_string),
    }
}
const C: AgentKind = AgentKind::ClaudeCode;

#[test]
fn rule_syntax_is_validated_and_never_silently_dropped() {
    for ok in [
        "Read",
        "Bash(git push:*)",
        "WebFetch(domain:example.com)",
        "mcp__srv__tool",
        "Read(~/.ssh/**)",
        "Bash(echo (a))",
    ] {
        assert!(validate_rule("allow", ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        " ",
        "Bash(",
        "Bash()",
        "Bash(x",
        "(x)",
        "1Bash",
        "Ba sh",
        "Bash(x)y",
        "Bash(a\nb)",
        "Bash(a\u{0}b)",
        "*",
        "Bash;rm",
    ] {
        assert!(validate_rule("allow", bad).is_err(), "{bad:?}");
    }
    assert!(validate_rule("allow", &format!("Bash({})", "a".repeat(301))).is_err());
    assert!(validate_rule("allow", &"A".repeat(65)).is_err());
    let err = validate_agent_permissions(C, p(&["Read", "bad rule"], &[], &[], None)).unwrap_err();
    assert!(format!("{err}").contains("bad rule"), "{err}");
    let many = AgentPermissions {
        allow: (0..201).map(|i| format!("T{i}")).collect(),
        ..Default::default()
    };
    assert!(validate_agent_permissions(C, many).is_err());
}

#[test]
fn normalises_dedupes_and_checks_modes() {
    let v = validate_agent_permissions(C, p(&[" Read ", "Read", "Grep"], &[], &[], Some(" plan ")))
        .unwrap();
    assert_eq!(v.allow, ["Read", "Grep"]);
    assert_eq!(v.default_mode.as_deref(), Some("plan"));
    assert_eq!(
        validate_agent_permissions(C, p(&[], &[], &[], Some("  ")))
            .unwrap()
            .default_mode,
        None
    );
    assert!(validate_agent_permissions(C, p(&[], &[], &[], Some("a b"))).is_err());
    assert!(validate_agent_permissions(C, p(&[], &[], &[], Some("x\ny"))).is_err());
    assert!(validate_agent_permissions(AgentKind::Antigravity, p(&[], &[], &[], None)).is_err());
}

#[test]
fn warnings_cover_conflicts_broad_rules_and_modes() {
    let w = permission_warnings(p(
        &["Bash", "Read", "A"],
        &["A"],
        &["Read"],
        Some("bypassPermissions"),
    ));
    let all = w.join("\n");
    assert!(all.contains("allows every shell command"), "{all}");
    assert!(all.contains("both allow and deny"), "{all}");
    assert!(all.contains("both allow and ask"), "{all}");
    assert!(all.contains("bypassPermissions"), "{all}");
    assert!(permission_warnings(p(&["Read"], &[], &[], Some("plan"))).is_empty());
    assert!(permission_warnings(p(&[], &[], &[], Some("zzz")))
        .join("")
        .contains("Unknown default mode"));
    assert!(permission_warnings(p(&[], &["X"], &["X"], None))
        .join("")
        .contains("ask and deny"));
}

#[test]
fn merge_preserves_everything_else_and_removes_empties() {
    let existing = r#"{"env":{"API_KEY":"sk-secret"},"model":"opus","permissions":{"allow":["Old"],"additionalDirectories":["../x"],"futureKey":1},"hooks":{"Stop":[]}}"#;
    let out = merge_permissions(
        C,
        existing.into(),
        p(&["Read"], &[], &["Read(./.env)"], Some("acceptEdits")),
    )
    .unwrap();
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["env"]["API_KEY"], "sk-secret");
    assert_eq!(v["model"], "opus");
    assert_eq!(v["hooks"]["Stop"], json!([]));
    assert_eq!(v["permissions"]["allow"], json!(["Read"]));
    assert_eq!(v["permissions"]["deny"], json!(["Read(./.env)"]));
    assert_eq!(v["permissions"]["defaultMode"], "acceptEdits");
    assert_eq!(v["permissions"]["additionalDirectories"], json!(["../x"]));
    assert_eq!(v["permissions"]["futureKey"], 1);
    assert!(v["permissions"].get("ask").is_none());
    // Emptying everything removes our keys, keeps the foreign ones, then the object itself.
    let cleared = merge_permissions(C, out.clone(), AgentPermissions::default()).unwrap();
    let v: Value = serde_json::from_str(&cleared).unwrap();
    assert_eq!(
        v["permissions"],
        json!({"additionalDirectories":["../x"],"futureKey":1})
    );
    let only = merge_permissions(
        C,
        r#"{"permissions":{"allow":["A"]}}"#.into(),
        AgentPermissions::default(),
    )
    .unwrap();
    assert!(serde_json::from_str::<Value>(&only)
        .unwrap()
        .get("permissions")
        .is_none());
    // A missing/empty file becomes a fresh object.
    let fresh = merge_permissions(C, "".into(), p(&["Read"], &[], &[], None)).unwrap();
    assert!(fresh.ends_with("}\n") && fresh.contains("\"Read\""));
}

#[test]
fn merge_and_read_refuse_unsafe_files() {
    let np = p(&["Read"], &[], &[], None);
    for bad in [
        "not json",
        "[1]",
        r#"{"permissions":[]}"#,
        r#"{"permissions":{},"permissions":{}}"#,
        r#"{"a":{"x":1,"x":2}}"#,
    ] {
        assert!(
            merge_permissions(C, bad.into(), np.clone()).is_err(),
            "{bad}"
        );
    }
    for bad in [
        r#"{"permissions":{"allow":"Read"}}"#,
        r#"{"permissions":{"allow":[1]}}"#,
        r#"{"permissions":{"allow":["bad rule"]}}"#,
        r#"{"permissions":{"deny":["Read\n(x)"]}}"#,
        r#"{"permissions":{"defaultMode":3}}"#,
        r#"{"permissions":{"defaultMode":"a b"}}"#,
        r#"{"permissions":5}"#,
    ] {
        assert!(permissions_from_settings(C, bad.into()).is_err(), "{bad}");
    }
    assert!(permissions_from_settings(AgentKind::Codex, "{}".into()).is_err());
    assert!(merge_permissions(AgentKind::Grok, "{}".into(), np).is_err());
}

#[test]
fn reads_the_four_fields_and_shows_only_the_permissions_section() {
    let json = r#"{"env":{"K":"secret"},"permissions":{"allow":["Read"," Grep "],"ask":["Bash(rm:*)"],"deny":[],"defaultMode":"plan"}}"#;
    let got = permissions_from_settings(C, json.into()).unwrap();
    assert_eq!(
        got,
        p(&["Read", "Grep"], &["Bash(rm:*)"], &[], Some("plan"))
    );
    assert_eq!(
        permissions_from_settings(C, "".into()).unwrap(),
        AgentPermissions::default()
    );
    assert_eq!(
        permissions_from_settings(C, "{}".into()).unwrap(),
        AgentPermissions::default()
    );
    let section = permissions_section_of(C, json.into()).unwrap();
    assert!(
        section.contains("\"allow\"") && !section.contains("secret") && !section.contains("env")
    );
    assert_eq!(permissions_section_of(C, "{}".into()).unwrap(), "{}");
}

#[test]
fn agent_config_is_protected_by_template_and_project_work() {
    let all = permission_rule_templates();
    let guard = all.iter().find(|t| t.id == "protect-agent-config").unwrap();
    for must in [
        "Edit(~/.claude/settings.json)",
        "Edit(~/.gemini/config/hooks.json)",
        "Edit(~/.codex/hooks.json)",
        "Edit(~/.grok/hooks/**)",
        "Edit(~/.ssh-app/**)",
    ] {
        assert!(guard.deny.contains(&must.to_string()), "{must}");
    }
    assert!(guard.allow.is_empty() && guard.ask.is_empty());
    let work = all.iter().find(|t| t.id == "project-work").unwrap();
    for rule in &guard.deny {
        assert!(work.deny.contains(rule), "project-work lacks {rule}");
    }
}

#[test]
fn templates_are_valid_rules() {
    for t in permission_rule_templates() {
        let v = validate_agent_permissions(
            C,
            AgentPermissions {
                allow: t.allow.clone(),
                ask: t.ask.clone(),
                deny: t.deny.clone(),
                default_mode: t.default_mode.clone(),
            },
        )
        .unwrap();
        assert_eq!(v.default_mode, t.default_mode, "{}", t.id);
        assert_eq!(
            (v.allow, v.ask, v.deny),
            (t.allow, t.ask, t.deny),
            "{}",
            t.id
        );
        assert!(!t.title.is_empty() && !t.description.is_empty());
    }
}

#[test]
fn suggestions_follow_what_is_installed() {
    let none = AgentPermissions::default();
    let ids = |p: AgentPermissions, ctx: Vec<AgentPermissions>| {
        suggest_permission_changes(p, ctx)
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>()
    };
    // Empty setup: the full set plus the smaller alternatives.
    let fresh = ids(none.clone(), vec![]);
    for id in [
        "project-work",
        "secrets",
        "agent-config",
        "destructive",
        "read-only",
    ] {
        assert!(fresh.contains(&id.to_string()), "{id}: {fresh:?}");
    }
    // Everything from the template already installed: nothing left to suggest.
    let work = permission_rule_templates()
        .into_iter()
        .find(|t| t.id == "project-work")
        .unwrap();
    let done = AgentPermissions {
        allow: work.allow.clone(),
        ask: work.ask.clone(),
        deny: work.deny.clone(),
        default_mode: work.default_mode.clone(),
    };
    assert!(
        ids(done.clone(), vec![]).is_empty(),
        "{:?}",
        ids(done.clone(), vec![])
    );
    // The same rules arriving from the project are not repeated, only the mode is missing.
    let from_project = AgentPermissions {
        default_mode: None,
        ..done.clone()
    };
    let s = suggest_permission_changes(none.clone(), vec![from_project]);
    let pw = s
        .iter()
        .find(|s| s.id == "project-work")
        .expect("mode suggestion");
    assert!(pw.add_allow.is_empty() && pw.add_ask.is_empty() && pw.add_deny.is_empty());
    assert_eq!(pw.default_mode.as_deref(), Some("acceptEdits"));
    // Risky and dead rules.
    let risky = AgentPermissions {
        allow: vec![
            "Bash".into(),
            "Bash(git:*)".into(),
            "Bash(git status:*)".into(),
            "Bash(rm:*)".into(),
        ],
        ask: vec!["Bash(rm:*)".into()],
        deny: vec![],
        default_mode: Some("bypassPermissions".into()),
    };
    let s = suggest_permission_changes(risky, vec![]);
    let get = |id: &str| {
        s.iter()
            .find(|x| x.id == id)
            .unwrap_or_else(|| panic!("{id}"))
    };
    assert_eq!(
        get("unattended").default_mode.as_deref(),
        Some("acceptEdits")
    );
    assert_eq!(get("broad-allow").remove_allow, vec!["Bash".to_string()]);
    assert_eq!(
        get("redundant").remove_allow,
        vec!["Bash(git status:*)".to_string()]
    );
    assert_eq!(
        get("dead-allow").remove_allow,
        vec!["Bash(rm:*)".to_string()]
    );
    // Every suggested rule is itself valid.
    for s in suggest_permission_changes(none, vec![]) {
        validate_agent_permissions(
            C,
            AgentPermissions {
                allow: s.add_allow,
                ask: s.add_ask,
                deny: s.add_deny,
                default_mode: s.default_mode,
            },
        )
        .unwrap();
    }
}

#[test]
fn lenient_reader_keeps_sane_rules_and_ignores_the_rest() {
    let p = lenient_permissions(r#"{"permissions":{"allow":["Read","a\nb",5,"Bash(ls:*)"],"defaultMode":"nonsense"},"env":{"K":"secret"}}"#).unwrap();
    assert_eq!(p.allow, vec!["Read".to_string(), "Bash(ls:*)".to_string()]);
    assert_eq!(p.default_mode, None);
    assert!(lenient_permissions("not json").is_none());
    assert!(lenient_permissions("{}").is_none());
}

#[test]
fn inspect_output_is_split_into_the_report() {
    let out = "==={T}:DIR===\n/work/app\n==={T}:CFG===\n/home/u/.claude-work\n==={T}:USER===\n{\"permissions\":{\"allow\":[\"Read\"],\"defaultMode\":\"plan\"}}\n==={T}:SHARED===\n\n==={T}:LOCAL===\n{\"permissions\":{\"allow\":[\"Bash(ls:*)\"]}}\n==={T}:END===\n".replace("{T}", "tag1");
    let r = parse_inspect_permissions_output(&out, "tag1");
    assert_eq!(r.directory.as_deref(), Some("/work/app"));
    assert_eq!(
        r.detected_config_dir.as_deref(),
        Some("/home/u/.claude-work")
    );
    assert_eq!(
        r.user.as_ref().unwrap().default_mode.as_deref(),
        Some("plan")
    );
    assert!(r.project_shared.is_none());
    assert_eq!(
        r.project_local.unwrap().allow,
        vec!["Bash(ls:*)".to_string()]
    );
    assert!(!r.suggestions.is_empty());
    // Nothing found at all still yields a report (no folder, no files).
    let empty = parse_inspect_permissions_output("", "tag1");
    assert!(empty.directory.is_none() && empty.user.is_none());
}

#[test]
fn a_settings_file_cannot_forge_section_markers() {
    // The shared project file (untrusted) tries to inject its own LOCAL section and END.
    let out = "==={T}:DIR===\n/w\n==={T}:CFG===\n\n==={T}:USER===\n\n==={T}:SHARED===\n===LOCAL===\n{\"permissions\":{\"allow\":[\"Bash(rm:*)\"]}}\n===END===\n==={T}:LOCAL===\n\n==={T}:END===\n"
            .replace("{T}", "tag1");
    let r = parse_inspect_permissions_output(&out, "tag1");
    assert!(r.project_local.is_none());
    // And the forged text is not treated as a marker even without the right tag.
    assert!(parse_inspect_permissions_output(&out, "other")
        .directory
        .is_none());
    assert_ne!(new_inspect_tag(), new_inspect_tag());
}

#[test]
fn checked_read_reports_a_bad_rule_instead_of_failing() {
    let json =
        "{\"permissions\":{\"allow\":[\"Read\"],\"deny\":[\"Bash(cd *\\n*)\",\"Bash(rm:*)\"]}}";
    assert!(permissions_from_settings(C, json.into()).is_err());
    let r = permissions_from_settings_checked(C, json.into()).unwrap();
    assert_eq!(r.permissions.allow, vec!["Read".to_string()]);
    assert_eq!(r.permissions.deny, vec!["Bash(rm:*)".to_string()]);
    assert_eq!(r.invalid.len(), 1);
    assert_eq!(r.invalid[0].list, "deny");
    assert_eq!(r.invalid[0].rule, "Bash(cd *\\n*)");
    // Real structural problems are still errors.
    assert!(
        permissions_from_settings_checked(C, "{\"permissions\":{\"allow\":5}}".into()).is_err()
    );
}
