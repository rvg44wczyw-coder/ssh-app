use super::*;

#[cfg(test)]
mod refresh_tests {
    use super::*;

    #[test]
    fn the_state_of_the_hook_script_names_the_code_that_runs() {
        let pin = "ab".repeat(32);
        // the script as shipped, with the pin slot filled with `p` (the shipped pin may be set)
        let ours = |p: &str| {
            let script = agent_hook_script();
            let at = script.find("HELPER_SHA256 = \"").unwrap() + "HELPER_SHA256 = \"".len();
            let end = at + script[at..].find('"').unwrap();
            format!("{}{p}{}", &script[..at], &script[end..])
        };
        assert_eq!(
            classify_hook_script("", Some(&pin)),
            HookScriptState::NotInstalled
        );
        assert_eq!(
            classify_hook_script("#!/bin/sh\n", Some(&pin)),
            HookScriptState::Foreign
        );
        assert_eq!(
            classify_hook_script(&ours(&pin), Some(&pin)),
            HookScriptState::Rust
        );
        assert_eq!(
            classify_hook_script(&ours(&"cd".repeat(32)), Some(&pin)),
            HookScriptState::Python
        );
        assert_eq!(
            classify_hook_script(&ours(&pin), None),
            HookScriptState::Python
        );
    }

    #[test]
    fn only_our_own_script_is_refreshed_without_asking() {
        let new = agent_hook_script();
        assert_eq!(
            plan_hook_script_refresh("", &new, false),
            HookScriptRefresh::NotInstalled
        );
        assert_eq!(
            plan_hook_script_refresh(&new, &new, false),
            HookScriptRefresh::UpToDate
        );
        // an older copy of ours with another pin
        let old = new.replacen("HELPER_SHA256 = \"", "HELPER_SHA256 = \"00", 1);
        assert!(is_our_hook_script(&old) || !new.contains("HELPER_SHA256 = \""));
        if is_our_hook_script(&old) {
            assert_eq!(
                plan_hook_script_refresh(&old, &new, false),
                HookScriptRefresh::Refreshed
            );
        }
        // somebody else's file at that path
        let foreign = "#!/bin/sh\necho my own hook\n";
        assert_eq!(
            plan_hook_script_refresh(foreign, &new, false),
            HookScriptRefresh::NeedsConfirmation
        );
        assert_eq!(
            plan_hook_script_refresh(foreign, &new, true),
            HookScriptRefresh::Refreshed
        );
        // a file that only mentions the name is not ours
        let mention = "#!/bin/sh\n# calls ssh-app agent hook\nexec other\n";
        assert_eq!(
            plan_hook_script_refresh(mention, &new, false),
            HookScriptRefresh::NeedsConfirmation
        );
    }
}

#[cfg(test)]
mod plan_tests {

    #[test]
    fn statusline_wraps_keeps_and_restores_the_users_command() {
        let none = merge_claude_statusline("{}".into(), None).unwrap();
        assert!(claude_statusline_installed(none.clone()));
        assert!(none.contains("claude statusline"));
        assert_eq!(merge_claude_statusline(none.clone(), None).unwrap(), none);
        assert_eq!(remove_claude_statusline(none).unwrap().trim(), "{}");

        let mine = r#"{"model":"x","statusLine":{"type":"command","command":"~/bin/sl.sh --a 'b'","padding":2}}"#;
        let wrapped = merge_claude_statusline(mine.into(), None).unwrap();
        let v: Value = serde_json::from_str(&wrapped).unwrap();
        assert_eq!(v["statusLine"]["padding"], 2);
        assert_eq!(v["model"], "x");
        // Installing twice must not wrap the wrapper.
        assert_eq!(
            merge_claude_statusline(wrapped.clone(), None).unwrap(),
            wrapped
        );
        let back: Value =
            serde_json::from_str(&remove_claude_statusline(wrapped).unwrap()).unwrap();
        assert_eq!(back["statusLine"]["command"], "~/bin/sl.sh --a 'b'");
        assert!(!claude_statusline_installed(back.to_string()));
    }

    #[test]
    fn statusline_refuses_what_it_cannot_wrap() {
        for bad in [
            r#"{"statusLine":{"type":"static","text":"x"}}"#,
            r#"{"statusLine":"x"}"#,
            "[1]",
        ] {
            assert!(merge_claude_statusline(bad.into(), None).is_err(), "{bad}");
        }
        let long = format!(
            r#"{{"statusLine":{{"type":"command","command":"{}"}}}}"#,
            "a".repeat(2001)
        );
        assert!(merge_claude_statusline(long, None).is_err());
    }
    use super::*;
    #[test]
    fn claude_merge_is_idempotent_and_preserves_user_settings() {
        let existing = r#"{"model":"opus","hooks":{"PermissionRequest":[{"matcher":"Bash","hooks":[{"type":"command","command":"/usr/local/bin/mine"}]}],"PreToolUse":[{"hooks":[{"type":"command","command":"x"}]}]}}"#;
        let once = merge_agent_hooks(AgentKind::ClaudeCode, existing.into(), None).unwrap();
        let twice = merge_agent_hooks(AgentKind::ClaudeCode, once.clone(), None).unwrap();
        assert_eq!(once, twice);
        let v: Value = serde_json::from_str(&once).unwrap();
        assert_eq!(v["model"], "opus");
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "x");
        let pr = v["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(pr.len(), 2);
        assert_eq!(pr[0]["hooks"][0]["command"], "/usr/local/bin/mine");
        assert_eq!(
            pr[1]["hooks"][0]["command"],
            "python3 -I \"$HOME/.ssh-app/bin/ssh-app-hook\" claude PermissionRequest"
        );
        assert_eq!(pr[1]["hooks"][0]["timeout"], 130);
        assert!(agent_hooks_installed(AgentKind::ClaudeCode, once));
    }

    #[test]
    fn claude_remove_restores_original_hooks() {
        let existing = r#"{"model":"opus","hooks":{"PermissionRequest":[{"matcher":"Bash","hooks":[{"type":"command","command":"mine"}]}]}}"#;
        let merged = merge_agent_hooks(AgentKind::ClaudeCode, existing.into(), None).unwrap();
        let removed = remove_agent_hooks(AgentKind::ClaudeCode, merged).unwrap();
        let a: Value = serde_json::from_str(existing).unwrap();
        let b: Value = serde_json::from_str(&removed).unwrap();
        assert_eq!(a, b);
        // From an empty config the `hooks` key disappears entirely.
        let m = merge_agent_hooks(AgentKind::ClaudeCode, String::new(), None).unwrap();
        let r = remove_agent_hooks(AgentKind::ClaudeCode, m).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&r).unwrap(), json!({}));
        assert!(!agent_hooks_installed(AgentKind::ClaudeCode, r));
    }

    #[test]
    fn antigravity_merge_and_remove() {
        let existing = r#"{"linter":{"PostToolUse":[{"matcher":"run_command","hooks":[{"type":"command","command":"./lint.sh"}]}]}}"#;
        let merged = merge_agent_hooks(
            AgentKind::Antigravity,
            existing.into(),
            Some("/opt/h".into()),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&merged).unwrap();
        assert!(v.get("linter").is_some());
        assert_eq!(
            v["ssh-app"]["PreToolUse"][0]["hooks"][0]["command"],
            "python3 -I \"/opt/h/ssh-app-hook\" antigravity PreToolUse"
        );
        assert_eq!(v["ssh-app"]["PreToolUse"][0]["matcher"], "run_command");
        assert_eq!(
            merge_agent_hooks(
                AgentKind::Antigravity,
                merged.clone(),
                Some("/opt/h".into())
            )
            .unwrap(),
            merged
        );
        assert!(agent_hooks_installed(
            AgentKind::Antigravity,
            merged.clone()
        ));
        let removed = remove_agent_hooks(AgentKind::Antigravity, merged).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&removed).unwrap(),
            serde_json::from_str::<Value>(existing).unwrap()
        );
    }

    #[test]
    fn invalid_or_non_object_config_is_never_overwritten() {
        for bad in ["{not json", "[1,2]", "\"str\""] {
            assert!(merge_agent_hooks(AgentKind::ClaudeCode, bad.into(), None).is_err());
            assert!(remove_agent_hooks(AgentKind::Antigravity, bad.into()).is_err());
            assert!(!agent_hooks_installed(AgentKind::ClaudeCode, bad.into()));
        }
        assert!(merge_agent_hooks(AgentKind::ClaudeCode, r#"{"hooks":[]}"#.into(), None).is_err());
        assert!(merge_agent_hooks(
            AgentKind::ClaudeCode,
            r#"{"hooks":{"Stop":{}}}"#.into(),
            None
        )
        .is_err());
    }

    #[test]
    fn merge_preserves_number_fidelity() {
        let existing = r#"{"big":18446744073709551616,"n":12345678901234567890123,"p":0.1000000000000000055511151231257827,"f":1.0}"#;
        let out = merge_agent_hooks(AgentKind::ClaudeCode, existing.into(), None).unwrap();
        for lit in [
            "18446744073709551616",
            "12345678901234567890123",
            "0.1000000000000000055511151231257827",
            "1.0",
        ] {
            assert!(out.contains(lit), "{lit} altered: {out}");
        }
    }

    #[test]
    fn codex_and_grok_hooks_follow_their_documented_formats() {
        let (c, g) = (AgentKind::Codex, AgentKind::Grok);
        assert_eq!(
            agent_hooks_config_path(c, None).unwrap(),
            "~/.codex/hooks.json"
        );
        assert_eq!(
            agent_hooks_config_path(c, Some("~/.codex-work".into())).unwrap(),
            "~/.codex-work/hooks.json"
        );
        assert_eq!(
            agent_hooks_config_path(g, None).unwrap(),
            "~/.grok/hooks/ssh-app.json"
        );
        // Codex: PermissionRequest (all tools) + Stop, grouped like Claude; user hooks survive.
        let existing =
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]}}"#;
        let plan = plan_agent_hooks(c, default_hook_preset(), None).unwrap();
        let out = merge_planned_hooks(c, existing.into(), plan).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hooks"]["PermissionRequest"][0]["matcher"], "*");
        assert!(v["hooks"]["PermissionRequest"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .ends_with("codex PermissionRequest"));
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(v["hooks"].get("Notification").is_none());
        let removed: Value = serde_json::from_str(&remove_agent_hooks(c, out).unwrap()).unwrap();
        assert_eq!(removed["hooks"]["Stop"].as_array().unwrap().len(), 1);
        // Grok: PreToolUse limited to Bash by default, plus Notification and Stop.
        let plan = plan_agent_hooks(g, default_hook_preset(), None).unwrap();
        let out = merge_planned_hooks(g, String::new(), plan).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        assert!(v["hooks"]["Notification"].is_array() && v["hooks"]["Stop"].is_array());
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["timeout"], 130);
        let events: Vec<_> = installed_hook_entries(g, out)
            .into_iter()
            .map(|h| h.event)
            .collect();
        assert_eq!(events, ["PreToolUse", "Notification", "Stop"]);
        // Presets: notify-only and custom events are validated per agent.
        let mut notify = default_hook_preset();
        notify.kind = HookPresetKind::NotifyOnly;
        assert_eq!(plan_agent_hooks(c, notify.clone(), None).unwrap().len(), 1);
        assert_eq!(plan_agent_hooks(g, notify, None).unwrap().len(), 2);
        assert!(plan_agent_hooks(
            c,
            approval(HookPresetKind::PhoneApproval, "", &["Notification"]),
            None
        )
        .is_err());
    }

    #[test]
    fn config_paths_follow_config_dir() {
        assert_eq!(
            agent_hooks_config_path(AgentKind::ClaudeCode, None).unwrap(),
            "~/.claude/settings.json"
        );
        assert_eq!(
            agent_hooks_config_path(AgentKind::ClaudeCode, Some("~/.claude-personal/".into()))
                .unwrap(),
            "~/.claude-personal/settings.json"
        );
        assert_eq!(
            agent_hooks_config_path(AgentKind::Antigravity, None).unwrap(),
            "~/.gemini/config/hooks.json"
        );
        assert!(agent_hooks_config_path(AgentKind::ClaudeCode, Some("a\nb".into())).is_err());
        assert!(
            merge_agent_hooks(AgentKind::ClaudeCode, String::new(), Some("a\nb".into())).is_err()
        );
    }

    #[test]
    fn hook_dir_cannot_inject_shell_into_the_agent_command() {
        for bad in [
            "/x\" ; id #",
            "/x$(id)",
            "/x`id`",
            "/x\\y",
            "/x'y",
            "/x;y",
            "/x|y",
            "/x&y",
            "/x\ty",
        ] {
            assert!(
                merge_agent_hooks(AgentKind::ClaudeCode, String::new(), Some(bad.into())).is_err(),
                "{bad}"
            );
            assert!(
                merge_agent_hooks(AgentKind::Antigravity, String::new(), Some(bad.into())).is_err(),
                "{bad}"
            );
        }
        assert!(merge_agent_hooks(
            AgentKind::ClaudeCode,
            String::new(),
            Some("/opt/my hooks/bin".into())
        )
        .is_ok());
    }

    #[test]
    fn write_command_and_script_path_are_quoted() {
        let c = build_write_file_command("\"$HOME\"'/x y'", "it's {}", "600");
        assert!(c.contains("printf %s 'it'\\''s {}'"));
        assert!(c.contains("chmod 600"));
        assert!(c.contains("rm -f \"$t\"") && c.contains("set -C"));
        assert_eq!(
            remote_script_path("~/.ssh-app/bin/"),
            "\"$HOME\"'/.ssh-app/bin/ssh-app-hook'"
        );
        assert!(agent_hook_script().contains("def main"));
        let pinned = agent_hook_script();
        match crate::usage::bundled_host_helper_sha256() {
            Some(pin) => assert!(pinned.contains(&format!("HELPER_SHA256 = \"{pin}\""))),
            None => assert!(pinned.contains("HELPER_SHA256 = \"\"")),
        }
    }

    #[test]
    fn antigravity_flat_events_are_written_read_and_removed_as_flat_handlers() {
        let a = AgentKind::Antigravity;
        let existing = r#"{"mine":{"Stop":[{"command":"./mine.sh"}]}}"#;
        let plan = plan_agent_hooks(a, default_hook_preset(), None).unwrap();
        let out = merge_planned_hooks(a, existing.into(), plan).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        // PreToolUse is grouped with its matcher; Stop is a flat list of handlers.
        assert_eq!(v["ssh-app"]["PreToolUse"][0]["matcher"], "run_command");
        assert!(v["ssh-app"]["PreToolUse"][0]["hooks"][0]["command"].is_string());
        assert!(v["ssh-app"]["Stop"][0].get("hooks").is_none());
        assert!(v["ssh-app"]["Stop"][0]["command"]
            .as_str()
            .unwrap()
            .contains("Stop"));
        let events: Vec<_> = installed_hook_entries(a, out.clone())
            .into_iter()
            .map(|h| h.event)
            .collect();
        assert_eq!(events, ["PreToolUse", "Stop"]);
        // Reinstall does not stack, removal keeps the user's own flat handler.
        let plan = plan_agent_hooks(a, default_hook_preset(), None).unwrap();
        let again = merge_planned_hooks(a, out, plan).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&again).unwrap()["ssh-app"]["Stop"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let removed = remove_agent_hooks(a, again).unwrap();
        let v: Value = serde_json::from_str(&removed).unwrap();
        assert!(v.get("ssh-app").is_none());
        assert_eq!(v["mine"]["Stop"][0]["command"], "./mine.sh");
    }

    fn approval(kind: HookPresetKind, matcher: &str, events: &[&str]) -> HookPreset {
        HookPreset {
            kind,
            approval_matcher: matcher.into(),
            approval_events: events.iter().map(|e| e.to_string()).collect(),
            ..default_hook_preset()
        }
    }

    #[test]
    fn approval_matcher_and_events_are_configurable_and_validated() {
        use HookPresetKind::{NotifyOnly, PhoneApproval};
        let plan = |a, p| plan_agent_hooks(a, p, None);
        let c = AgentKind::ClaudeCode;
        let p = plan(
            c,
            approval(PhoneApproval, " Bash|Edit ", &["Stop", "PermissionRequest"]),
        )
        .unwrap();
        let got: Vec<_> = p
            .iter()
            .map(|h| (h.event.as_str(), h.matcher.as_deref()))
            .collect();
        assert_eq!(
            got,
            [("PermissionRequest", Some("Bash|Edit")), ("Stop", None)]
        );
        // Defaults are unchanged when nothing is chosen.
        let d = plan(c, default_hook_preset()).unwrap();
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].matcher.as_deref(), Some("*"));
        // Notify-only has no tool event, so the matcher is irrelevant and PermissionRequest is refused.
        assert_eq!(plan(c, approval(NotifyOnly, "Bash", &[])).unwrap().len(), 2);
        assert!(plan(c, approval(NotifyOnly, "", &["PermissionRequest"])).is_err());
        // Antigravity keeps its own default matcher and events.
        let a = plan(
            AgentKind::Antigravity,
            approval(PhoneApproval, "", &["PreToolUse"]),
        )
        .unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].matcher.as_deref(), Some(ANTIGRAVITY_TOOL_MATCHER));
        assert!(plan(
            AgentKind::Antigravity,
            approval(PhoneApproval, "", &["Notification"])
        )
        .is_err());
        // Hostile values are rejected, never written.
        for bad in ["Bash;rm", "a b", "x\ny", "\"", &"a".repeat(201)] {
            assert!(
                plan(c, approval(PhoneApproval, bad, &[])).is_err(),
                "{bad:?}"
            );
        }
        assert!(plan(c, approval(PhoneApproval, "", &["Stop\n", "Bogus"])).is_err());
        // Duplicates collapse.
        assert_eq!(
            plan(c, approval(PhoneApproval, "", &["Stop", "Stop"]))
                .unwrap()
                .len(),
            1
        );
    }

    fn custom(cmd: &str, events: &[&str], matcher: &str) -> HookPreset {
        HookPreset {
            kind: HookPresetKind::Custom,
            custom_command: cmd.into(),
            custom_events: events.iter().map(|e| e.to_string()).collect(),
            custom_matcher: matcher.into(),
            approval_matcher: String::new(),
            approval_events: vec![],
        }
    }

    #[test]
    fn presets_plan_expected_events() {
        let ev = |a, k| {
            let mut p = default_hook_preset();
            p.kind = k;
            plan_agent_hooks(a, p, None)
                .unwrap()
                .into_iter()
                .map(|h| h.event)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ev(AgentKind::ClaudeCode, HookPresetKind::PhoneApproval),
            ["PermissionRequest", "Notification", "Stop"]
        );
        assert_eq!(
            ev(AgentKind::ClaudeCode, HookPresetKind::NotifyOnly),
            ["Notification", "Stop"]
        );
        assert_eq!(
            ev(AgentKind::Antigravity, HookPresetKind::NotifyOnly),
            ["Stop"]
        );
    }

    #[test]
    fn custom_preset_is_marked_deduped_and_removable() {
        let p = custom(
            "/usr/local/bin/mine --flag",
            &["PreToolUse", "Stop", "Stop"],
            "Bash|Edit",
        );
        let plan = plan_agent_hooks(AgentKind::ClaudeCode, p.clone(), None).unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].matcher.as_deref(), Some("Bash|Edit"));
        assert_eq!(plan[1].matcher, None);
        assert!(plan[0].command.ends_with(" # ssh-app-hook-custom"));
        assert!(!hook_preset_uses_script(p));

        let merged =
            merge_planned_hooks(AgentKind::ClaudeCode, "{\"theme\":1}".into(), plan).unwrap();
        assert!(agent_hooks_installed(AgentKind::ClaudeCode, merged.clone()));
        let cleaned = remove_agent_hooks(AgentKind::ClaudeCode, merged).unwrap();
        assert!(!agent_hooks_installed(
            AgentKind::ClaudeCode,
            cleaned.clone()
        ));
        assert!(cleaned.contains("theme"));
    }

    #[test]
    fn custom_preset_validation() {
        let plan = |p| plan_agent_hooks(AgentKind::Antigravity, p, None);
        assert!(plan(custom("", &["Stop"], "")).is_err());
        assert!(plan(custom("x\ny", &["Stop"], "")).is_err());
        assert!(plan(custom("x", &[], "")).is_err());
        assert!(plan(custom("x", &["Notification"], "")).is_err());
        assert!(plan(custom("x", &["Stop"], "a b;")).is_err());
        let ok = plan(custom("x", &["PreToolUse"], "")).unwrap();
        assert_eq!(ok[0].matcher.as_deref(), Some("run_command"));
    }

    #[test]
    fn hooks_section_hides_foreign_settings_and_lists_installed() {
        let existing = r#"{"env":{"ANTHROPIC_API_KEY":"sk-secret"},"apiKeyHelper":"x",
            "hooks":{"Stop":[{"hooks":[{"type":"command","command":"theirs"}]}]}}"#;
        let plan = plan_agent_hooks(AgentKind::ClaudeCode, default_hook_preset(), None).unwrap();
        let merged = merge_planned_hooks(AgentKind::ClaudeCode, existing.into(), plan).unwrap();
        let section = hooks_section_of(AgentKind::ClaudeCode, merged.clone()).unwrap();
        assert!(!section.contains("sk-secret") && !section.contains("apiKeyHelper"));
        assert!(section.contains("theirs") && section.contains("PermissionRequest"));
        let installed = installed_hook_entries(AgentKind::ClaudeCode, merged);
        assert_eq!(installed.len(), 3);
        assert!(installed.iter().all(|h| h.command.contains(HOOK_MARKER)));
        assert!(!installed.iter().any(|h| h.command == "theirs"));
        assert!(installed_hook_entries(AgentKind::ClaudeCode, "not json".into()).is_empty());
    }

    #[test]
    fn antigravity_installed_entries_round_trip() {
        let plan = plan_agent_hooks(AgentKind::Antigravity, default_hook_preset(), None).unwrap();
        let merged =
            merge_planned_hooks(AgentKind::Antigravity, "{}".into(), plan.clone()).unwrap();
        assert_eq!(installed_hook_entries(AgentKind::Antigravity, merged), plan);
    }

    #[test]
    fn removal_keeps_foreign_siblings_and_lookalike_paths() {
        let existing = r#"{"hooks":{"Stop":[
          {"hooks":[{"type":"command","command":"theirs-sibling"},
                    {"type":"command","command":"python3 -I \"$HOME/.ssh-app/bin/ssh-app-hook\" claude Stop"}]},
          {"hooks":[{"type":"command","command":"/home/u/code/ssh-app-hook-tests/run.sh"}]},
          {"hooks":[]}]}}"#;
        let out = remove_agent_hooks(AgentKind::ClaudeCode, existing.into()).unwrap();
        assert!(out.contains("theirs-sibling"), "{out}");
        assert!(out.contains("ssh-app-hook-tests/run.sh"), "{out}");
        assert!(!out.contains("claude Stop"), "{out}");
        assert!(
            out.contains("\"hooks\": []"),
            "pre-existing empty entry stays: {out}"
        );
        assert!(!agent_hooks_installed(AgentKind::ClaudeCode, out));
    }

    #[test]
    fn antigravity_keeps_foreign_handlers_under_the_same_key() {
        let existing =
            r#"{"ssh-app":{"PreToolUse":[{"hooks":[{"command":"mine-own-named-hook"}]}]}}"#;
        let plan = plan_agent_hooks(AgentKind::Antigravity, default_hook_preset(), None).unwrap();
        let merged = merge_planned_hooks(AgentKind::Antigravity, existing.into(), plan).unwrap();
        assert!(merged.contains("mine-own-named-hook") && merged.contains("antigravity Stop"));
        let rm = remove_agent_hooks(AgentKind::Antigravity, merged).unwrap();
        assert!(rm.contains("mine-own-named-hook") && !rm.contains("antigravity"));
    }

    #[test]
    fn duplicate_keys_are_refused_not_silently_dropped() {
        let dup = r#"{"model":"a","hooks":{},"model":"b"}"#;
        let err = merge_agent_hooks(AgentKind::ClaudeCode, dup.into(), None).unwrap_err();
        assert!(err.to_string().contains("duplicate key"), "{err}");
        let nested = r#"{"hooks":{"Stop":[],"Stop":[]}}"#;
        assert!(remove_agent_hooks(AgentKind::ClaudeCode, nested.into()).is_err());
        // Same key in different objects and keys inside strings are fine.
        let ok = r#"{"a":{"x":1},"b":{"x":"{\"x\":1,\"x\":2}"},"c":[{"x":1},{"x":2}]}"#;
        assert!(merge_agent_hooks(AgentKind::ClaudeCode, ok.into(), None).is_ok());
    }

    fn plan(agent: AgentKind) -> Vec<PlannedHook> {
        plan_agent_hooks(agent, default_hook_preset(), None).unwrap()
    }

    #[test]
    fn qwen_uses_claude_style_grouped_hooks_in_its_settings() {
        assert_eq!(
            agent_hooks_config_path(AgentKind::QwenCode, None).unwrap(),
            "~/.qwen/settings.json"
        );
        let p = plan(AgentKind::QwenCode);
        let events: Vec<_> = p.iter().map(|h| h.event.as_str()).collect();
        assert_eq!(events, ["PermissionRequest", "Notification", "Stop"]);
        assert!(p[0]
            .command
            .ends_with("ssh-app-hook\" qwen PermissionRequest"));
        let merged = merge_planned_hooks(
            AgentKind::QwenCode,
            r#"{"theme":"dark","hooks":{"PreToolUse":[{"matcher":"x","hooks":[{"type":"command","command":"mine"}]}]}}"#.into(),
            p.clone(),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(
            v["hooks"]["PermissionRequest"][0]["hooks"][0]["type"],
            "command"
        );
        assert_eq!(
            installed_hook_entries(AgentKind::QwenCode, merged.clone()),
            p
        );
        let removed = remove_agent_hooks(AgentKind::QwenCode, merged).unwrap();
        let v: Value = serde_json::from_str(&removed).unwrap();
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "mine");
        assert!(v["hooks"].get("PermissionRequest").is_none());
    }

    #[test]
    fn crush_gets_flat_pre_tool_use_entries_with_an_anchored_matcher() {
        let p = plan(AgentKind::Crush);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].event, "PreToolUse");
        assert_eq!(p[0].matcher.as_deref(), Some("^bash$"));
        let merged = merge_planned_hooks(
            AgentKind::Crush,
            r#"{"models":{"a":1},"hooks":{"PreToolUse":[{"command":"mine","matcher":"^edit$"}]}}"#
                .into(),
            p.clone(),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&merged).unwrap();
        let list = v["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1]["matcher"], "^bash$");
        assert_eq!(list[1]["timeout"], HOOK_TIMEOUT_SECS);
        assert!(list[1].get("hooks").is_none() && list[1].get("type").is_none());
        // What is installed equals the plan: no drift is reported.
        let installed = installed_hook_entries(AgentKind::Crush, merged.clone());
        assert_eq!(installed, p);
        assert!(hooks_drift(p, installed).is_empty());
        // Removal takes only ours and keeps the user's own hook and the rest of the file.
        let v: Value =
            serde_json::from_str(&remove_agent_hooks(AgentKind::Crush, merged).unwrap()).unwrap();
        assert_eq!(v["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
        assert_eq!(v["models"]["a"], 1);
    }

    #[test]
    fn crush_and_opencode_matchers_are_tool_name_lists() {
        let preset = |m: &str| HookPreset {
            approval_matcher: m.into(),
            ..default_hook_preset()
        };
        let crush = plan_agent_hooks(AgentKind::Crush, preset("bash|edit"), None).unwrap();
        assert_eq!(crush[0].matcher.as_deref(), Some("^(bash|edit)$"));
        let all = plan_agent_hooks(AgentKind::Crush, preset("*"), None).unwrap();
        assert_eq!(all[0].matcher.as_deref(), Some(".*"));
        for bad in ["a.b", "a:b", "(x)", "$"] {
            assert!(
                plan_agent_hooks(AgentKind::Crush, preset(bad), None).is_err(),
                "{bad}"
            );
            assert!(
                plan_agent_hooks(AgentKind::OpenCode, preset(bad), None).is_err(),
                "{bad}"
            );
        }
        let custom = HookPreset {
            kind: HookPresetKind::Custom,
            custom_command: "true".into(),
            custom_events: vec!["PreToolUse".into()],
            ..default_hook_preset()
        };
        assert!(plan_agent_hooks(AgentKind::Crush, custom.clone(), None).is_err());
        assert!(plan_agent_hooks(AgentKind::OpenCode, custom, None).is_err());
        let notify = HookPreset {
            kind: HookPresetKind::NotifyOnly,
            ..default_hook_preset()
        };
        assert!(plan_agent_hooks(AgentKind::Crush, notify, None).is_err());
    }

    #[test]
    fn opencode_plugin_blocks_by_throwing_and_round_trips() {
        assert_eq!(
            agent_hooks_config_path(AgentKind::OpenCode, None).unwrap(),
            "~/.config/opencode/plugins/ssh-app.js"
        );
        let p = plan(AgentKind::OpenCode);
        let text = merge_planned_hooks(AgentKind::OpenCode, String::new(), p.clone()).unwrap();
        assert!(text.contains("\"tool.execute.before\""));
        assert!(text.contains("throw new Error"));
        assert!(text.contains("execFile(\"python3\""), "no shell: {text}");
        assert!(text.contains("join(homedir(), \".ssh-app/bin/ssh-app-hook\")"));
        assert!(text.contains("^(?:bash)$"));
        assert!(text.contains("session.idle"));
        assert_eq!(installed_hook_entries(AgentKind::OpenCode, text.clone()), p);
        // Idempotent, and removal leaves an inert plugin of ours (no tool hook, no entries).
        assert_eq!(
            merge_planned_hooks(AgentKind::OpenCode, text.clone(), p).unwrap(),
            text
        );
        let removed = remove_agent_hooks(AgentKind::OpenCode, text).unwrap();
        assert!(!removed.contains("tool.execute.before"));
        assert!(installed_hook_entries(AgentKind::OpenCode, removed).is_empty());
    }

    #[test]
    fn opencode_generator_refuses_hostile_entries_and_escapes_line_separators() {
        let ok = plan(AgentKind::OpenCode);
        let with = |f: &dyn Fn(&mut PlannedHook)| {
            let mut p = ok.clone();
            f(&mut p[0]);
            merge_planned_hooks(AgentKind::OpenCode, String::new(), p)
        };
        assert!(with(&|h| h.command.push_str("\u{2028}globalThis.x=1;//")).is_err());
        assert!(with(&|h| h.command.push_str("\u{2029}x")).is_err());
        assert!(with(&|h| h.command.push('\n')).is_err());
        assert!(with(&|h| h.command = "node evil.js".into()).is_err());
        assert!(with(&|h| h.event = "PostToolUse".into()).is_err());
        assert!(with(&|h| h.matcher = Some("(a+)+$".into())).is_err());
        assert!(with(&|h| h.matcher = Some("a\"+process.exit()+\"".into())).is_err());
        assert!(with(&|h| h.matcher = Some("bash|edit".into())).is_ok());
        // The header line stays one line whatever the (validated) content is.
        let text = merge_planned_hooks(AgentKind::OpenCode, String::new(), ok).unwrap();
        assert!(!text
            .lines()
            .next()
            .unwrap()
            .contains(['\u{2028}', '\u{2029}']));
    }

    #[test]
    fn opencode_never_overwrites_a_foreign_plugin_file() {
        let foreign = "export const Mine = async () => ({})\n";
        let p = plan(AgentKind::OpenCode);
        assert!(merge_planned_hooks(AgentKind::OpenCode, foreign.into(), p).is_err());
        assert!(remove_agent_hooks(AgentKind::OpenCode, foreign.into()).is_err());
        assert!(installed_hook_entries(AgentKind::OpenCode, foreign.into()).is_empty());
    }

    #[test]
    fn opencode_custom_hook_dir_is_used_as_an_absolute_path() {
        let p = plan_agent_hooks(
            AgentKind::OpenCode,
            default_hook_preset(),
            Some("/opt/hooks".into()),
        )
        .unwrap();
        let text = merge_planned_hooks(AgentKind::OpenCode, String::new(), p).unwrap();
        assert!(
            text.contains("const SCRIPT = \"/opt/hooks/ssh-app-hook\""),
            "{text}"
        );
    }

    #[test]
    fn agents_without_hooks_return_typed_errors_everywhere() {
        for a in [
            AgentKind::Cline,
            AgentKind::Aider,
            AgentKind::Goose,
            AgentKind::OllamaChat,
        ] {
            assert!(!a.has_hooks());
            assert!(agent_hooks_config_path(a, None).is_err());
            assert!(plan_agent_hooks(a, default_hook_preset(), None).is_err());
            assert!(merge_agent_hooks(a, "{}".into(), None).is_err());
            assert!(remove_agent_hooks(a, "{}".into()).is_err());
            assert!(hooks_section_of(a, "{}".into()).is_err());
            assert!(installed_hook_entries(a, "{}".into()).is_empty());
            assert!(!agent_hooks_installed(a, "{}".into()));
            assert!(hook_events_for(a).is_empty());
            assert!(crate::host_guard::host_guard_agent_support(a).is_empty());
        }
    }

    #[test]
    fn custom_command_must_keep_the_marker_a_comment() {
        let plan =
            |c: &str| plan_agent_hooks(AgentKind::ClaudeCode, custom(c, &["Stop"], ""), None);
        for bad in [
            "echo hi \\",
            "foo 'bar",
            "foo \"bar",
            "echo (",
            "echo )",
            "a |",
            "a &&",
            "echo $(x",
        ] {
            assert!(plan(bad).is_err(), "{bad:?} should be rejected");
        }
        for good in [
            "echo 'a b' \"c d\"",
            "x | y",
            "audit --log \\; true",
            "echo $(date)",
            "a &",
            "echo \\'",
        ] {
            assert!(plan(good).is_ok(), "{good:?} should be accepted");
        }
    }
}
