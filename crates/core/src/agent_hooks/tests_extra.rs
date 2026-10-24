//! Per-event extra and own commands, the Off kind and the event texts (feature 57).
use super::*;

fn preset(kind: HookPresetKind) -> HookPreset {
    HookPreset {
        kind,
        ..default_hook_preset()
    }
}

fn extra(event: &str, command: &str) -> HookExtra {
    HookExtra {
        event: event.into(),
        command: command.into(),
        order: HookExtraOrder::Before,
        stop_on_failure: false,
        timeout_secs: 10,
    }
}

fn own(event: &str, command: &str) -> HookOwn {
    HookOwn {
        event: event.into(),
        command: command.into(),
        matcher: String::new(),
    }
}

const C: AgentKind = AgentKind::ClaudeCode;

#[test]
fn an_extra_wraps_only_its_event_and_keeps_the_hook_command_inside() {
    let mut p = preset(HookPresetKind::PhoneApproval);
    p.extras = vec![extra("Stop", "echo done >> /tmp/log")];
    let plan = plan_agent_hooks(C, p, None).unwrap();
    let stop = plan.iter().find(|h| h.event == "Stop").unwrap();
    assert!(stop.command.starts_with("python3 -I \""));
    assert!(stop.command.contains("/ssh-app-hook-extra\" --cmd "));
    assert!(stop
        .command
        .ends_with("-- python3 -I \"$HOME/.ssh-app/bin/ssh-app-hook\" claude Stop"));
    assert!(
        !stop.command.contains("echo done"),
        "the user's text is base64, never visible as shell"
    );
    let plain = plan.iter().find(|h| h.event == "Notification").unwrap();
    assert_eq!(
        plain.command,
        "python3 -I \"$HOME/.ssh-app/bin/ssh-app-hook\" claude Notification"
    );
    assert!(is_our_command(&stop.command), "uninstall must recognise it");
}

#[test]
fn the_summary_shows_the_decoded_command_and_its_settings() {
    let mut p = preset(HookPresetKind::PhoneApproval);
    p.extras = vec![HookExtra {
        stop_on_failure: true,
        timeout_secs: 7,
        ..extra("PermissionRequest", "./check.sh --strict")
    }];
    let plan = plan_agent_hooks(C, p, None).unwrap();
    let h = plan
        .iter()
        .find(|h| h.event == "PermissionRequest")
        .unwrap();
    let text = hook_command_summary(h.command.clone());
    assert!(text.contains("./check.sh --strict"), "{text}");
    assert!(
        text.contains("before the hook, stops on failure, 7 s"),
        "{text}"
    );
    assert_eq!(hook_command_summary("ls -l".into()), "ls -l");
    assert_eq!(
        hook_command_summary("true # ssh-app-hook-custom".into()),
        "own command: true"
    );
}

#[test]
fn extras_are_validated() {
    let plan = |x: Vec<HookExtra>| {
        let mut p = preset(HookPresetKind::PhoneApproval);
        p.extras = x;
        plan_agent_hooks(C, p, None)
    };
    assert!(
        plan(vec![extra("PostToolUse", "true")]).is_err(),
        "not an installed event"
    );
    assert!(
        plan(vec![extra("Stop", "true"), extra("Stop", "false")]).is_err(),
        "two on one event"
    );
    assert!(plan(vec![extra("Stop", "")]).is_err());
    assert!(plan(vec![extra("Stop", "a\nb")]).is_err());
    assert!(plan(vec![extra("Stop", &"x".repeat(1001))]).is_err());
    assert!(plan(vec![HookExtra {
        timeout_secs: 0,
        ..extra("Stop", "true")
    }])
    .is_err());
    assert!(plan(vec![HookExtra {
        timeout_secs: 61,
        ..extra("Stop", "true")
    }])
    .is_err());
    assert!(
        plan(vec![HookExtra {
            stop_on_failure: true,
            ..extra("Stop", "true")
        }])
        .is_err(),
        "Stop cannot block"
    );
    assert!(
        plan(vec![HookExtra {
            stop_on_failure: true,
            order: HookExtraOrder::After,
            ..extra("PermissionRequest", "true")
        }])
        .is_err(),
        "stopping needs the command first"
    );
    assert!(plan(vec![HookExtra {
        stop_on_failure: true,
        ..extra("PermissionRequest", "true")
    }])
    .is_ok());
    // A quote or substitution in the command is data, not shell.
    assert!(plan(vec![extra("Stop", "echo \"$(id)\" ' ; rm -rf ~ #")]).is_ok());
}

#[test]
fn an_own_command_takes_an_event_the_preset_does_not_install() {
    let mut p = preset(HookPresetKind::NotifyOnly);
    p.own = vec![own("PostToolUse", "./log-tool.sh")];
    let plan = plan_agent_hooks(C, p, None).unwrap();
    let mine = plan.iter().find(|h| h.event == "PostToolUse").unwrap();
    assert_eq!(mine.command, "./log-tool.sh # ssh-app-hook-custom");
    assert_eq!(
        mine.matcher.as_deref(),
        Some("*"),
        "tool events get the default matcher"
    );
    assert!(is_our_command(&mine.command));
    assert!(
        plan.iter().any(|h| h.event == "Stop"),
        "the other events keep the hook"
    );

    let mut clash = preset(HookPresetKind::PhoneApproval);
    clash.own = vec![own("Stop", "true")];
    assert!(
        plan_agent_hooks(C, clash, None).is_err(),
        "Stop is the hook's"
    );
    let mut unknown = preset(HookPresetKind::NotifyOnly);
    unknown.own = vec![own("Nope", "true")];
    assert!(plan_agent_hooks(C, unknown, None).is_err());
    let mut twice = preset(HookPresetKind::NotifyOnly);
    twice.own = vec![own("PostToolUse", "true"), own("PostToolUse", "false")];
    assert!(plan_agent_hooks(C, twice, None).is_err());
    let mut open_quote = preset(HookPresetKind::NotifyOnly);
    open_quote.own = vec![own("PostToolUse", "echo 'oops")];
    assert!(plan_agent_hooks(C, open_quote, None).is_err());
}

#[test]
fn off_installs_only_the_own_commands() {
    let none = plan_agent_hooks(C, preset(HookPresetKind::Off), None).unwrap();
    assert!(none.is_empty());
    let mut p = preset(HookPresetKind::Off);
    p.own = vec![own("Stop", "./done.sh"), own("PostToolUse", "./t.sh")];
    let plan = plan_agent_hooks(C, p.clone(), None).unwrap();
    assert_eq!(plan.len(), 2);
    assert!(!hook_preset_uses_script(p.clone()));
    assert!(!hook_preset_uses_extra(p));
    let mut with_extra = preset(HookPresetKind::Off);
    with_extra.extras = vec![extra("Stop", "true")];
    assert!(
        plan_agent_hooks(C, with_extra, None).is_err(),
        "an extra needs the hook"
    );
}

#[test]
fn a_custom_hook_does_not_combine_with_per_event_commands() {
    let mut p = preset(HookPresetKind::Custom);
    p.custom_command = "true".into();
    p.custom_events = vec!["Stop".into()];
    assert!(plan_agent_hooks(C, p.clone(), None).is_ok());
    p.own = vec![own("PostToolUse", "true")];
    assert!(plan_agent_hooks(C, p, None).is_err());
}

#[test]
fn crush_and_opencode_refuse_extra_and_own_commands() {
    for agent in [AgentKind::Crush, AgentKind::OpenCode] {
        let mut p = preset(HookPresetKind::PhoneApproval);
        p.extras = vec![extra("PreToolUse", "true")];
        assert!(plan_agent_hooks(agent, p, None).is_err(), "{agent:?}");
        let mut q = preset(HookPresetKind::PhoneApproval);
        q.own = vec![own("PreToolUse", "true")];
        assert!(plan_agent_hooks(agent, q, None).is_err(), "{agent:?}");
    }
}

#[test]
fn wrapped_entries_install_list_and_remove_like_any_other() {
    let mut p = preset(HookPresetKind::PhoneApproval);
    p.extras = vec![extra("Stop", "true")];
    p.own = vec![own("PostToolUse", "./t.sh")];
    let plan = plan_agent_hooks(C, p.clone(), None).unwrap();
    let merged = merge_planned_hooks(C, "{\"model\":\"x\"}".into(), plan.clone()).unwrap();
    assert!(hook_preset_uses_extra(p));
    let listed = installed_hook_entries(C, merged.clone());
    for h in &plan {
        assert!(listed.contains(h), "{h:?} must be found again");
    }
    assert!(hooks_drift(plan, listed).is_empty());
    let removed = remove_agent_hooks(C, merged).unwrap();
    assert!(installed_hook_entries(C, removed.clone()).is_empty());
    assert!(removed.contains("\"model\""), "other settings stay");
}

#[test]
fn every_event_of_every_agent_has_a_text_and_blocking_flags_are_right() {
    for agent in [
        AgentKind::ClaudeCode,
        AgentKind::Antigravity,
        AgentKind::Codex,
        AgentKind::Grok,
        AgentKind::QwenCode,
        AgentKind::Crush,
        AgentKind::OpenCode,
    ] {
        for event in hook_events_for(agent) {
            let info = hook_event_info(agent, event.clone());
            assert!(
                !info.title.is_empty() && info.title != "Event",
                "{agent:?} {event}"
            );
            assert!(info.summary.len() > 10);
            assert_eq!(
                info.can_block,
                matches!(event.as_str(), "PermissionRequest" | "PreToolUse")
            );
        }
    }
    assert!(hook_event_info(C, "PostToolUse".into()).takes_matcher);
    assert!(!hook_event_info(C, "Stop".into()).takes_matcher);
}

#[test]
fn the_wrapper_script_is_embedded_and_not_the_hook() {
    let w = agent_hook_extra_script();
    assert!(w.contains("ssh-app hook extra-command wrapper"));
    assert!(
        !w.contains("HELPER_SHA256"),
        "the wrapper must not look like the hook script"
    );
    assert!(!is_our_hook_script(&w));
}

#[test]
fn the_agents_time_limit_covers_the_extra_command_and_the_hooks_own_wait() {
    let plain = hook_command("~/.ssh-app/bin", "claude", "PreToolUse");
    assert_eq!(handler(plain)["timeout"], HOOK_TIMEOUT_SECS);
    let mut x = extra("PreToolUse", "true");
    x.timeout_secs = 25;
    let wrapped = extra_command("~/.ssh-app/bin", "claude", "PreToolUse", &x);
    assert_eq!(handler(wrapped)["timeout"], HOOK_TIMEOUT_SECS + 25);
}
