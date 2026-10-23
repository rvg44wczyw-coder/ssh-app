use super::*;
use crate::agents::AgentKind;
use crate::skills::{
    agent_name, build_command, build_request, parse_document, parse_reply, parse_summaries,
    SkillDocument, SkillSummary,
};
use serde_json::{json, Value};

impl SshSessionHandle {
    async fn skills_call(
        &self,
        helper_sha256: &str,
        op: &str,
        agent: AgentKind,
        project_dir: &str,
        name: Option<&str>,
        extra: Value,
    ) -> Result<Value, SshCoreError> {
        crate::usage::validate_sha256(helper_sha256)?;
        let request = build_request(op, agent, project_dir, name, extra)?;
        let out = self
            .execute_command_on_active_session(build_command(helper_sha256, &request))
            .await?;
        parse_reply(&out)
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
    /// Skills of the project for this agent (project ones editable, user ones read only).
    pub async fn list_skills(
        &self,
        agent: AgentKind,
        project_dir: String,
        helper_sha256: String,
    ) -> Result<Vec<SkillSummary>, SshCoreError> {
        let v = self
            .skills_call(&helper_sha256, "list", agent, &project_dir, None, json!({}))
            .await?;
        Ok(parse_summaries(&v))
    }

    pub async fn read_skill(
        &self,
        agent: AgentKind,
        project_dir: String,
        name: String,
        user_scope: bool,
        helper_sha256: String,
    ) -> Result<SkillDocument, SshCoreError> {
        let extra = if user_scope {
            json!({"scope": "user"})
        } else {
            json!({})
        };
        let v = self
            .skills_call(
                &helper_sha256,
                "read",
                agent,
                &project_dir,
                Some(&name),
                extra,
            )
            .await?;
        Ok(parse_document(&v))
    }

    /// Saves a project skill: `create` makes a new one (it must not exist), otherwise it replaces
    /// the text of an existing one (the old text stays as `SKILL.md.bak`).
    pub async fn save_skill(
        &self,
        agent: AgentKind,
        project_dir: String,
        name: String,
        content: String,
        create: bool,
        helper_sha256: String,
    ) -> Result<(), SshCoreError> {
        let op = if create { "create" } else { "write" };
        self.skills_call(
            &helper_sha256,
            op,
            agent,
            &project_dir,
            Some(&name),
            json!({"content": content}),
        )
        .await
        .map(|_| ())
    }

    /// Moves a project skill to the trash folder; returns the trash entry to restore it by.
    pub async fn delete_skill(
        &self,
        agent: AgentKind,
        project_dir: String,
        name: String,
        helper_sha256: String,
    ) -> Result<String, SshCoreError> {
        let v = self
            .skills_call(
                &helper_sha256,
                "delete",
                agent,
                &project_dir,
                Some(&name),
                json!({}),
            )
            .await?;
        Ok(v.get("trash")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    pub async fn restore_skill(
        &self,
        agent: AgentKind,
        project_dir: String,
        trash_entry: String,
        helper_sha256: String,
    ) -> Result<(), SshCoreError> {
        self.skills_call(
            &helper_sha256,
            "restore",
            agent,
            &project_dir,
            Some(&trash_entry),
            json!({}),
        )
        .await
        .map(|_| ())
    }

    pub async fn set_skill_enabled(
        &self,
        agent: AgentKind,
        project_dir: String,
        name: String,
        enabled: bool,
        helper_sha256: String,
    ) -> Result<(), SshCoreError> {
        self.skills_call(
            &helper_sha256,
            "set_enabled",
            agent,
            &project_dir,
            Some(&name),
            json!({"enabled": enabled}),
        )
        .await
        .map(|_| ())
    }

    /// Trash entries (`<skill>-<unix time>`), kept 30 days.
    pub async fn list_skill_trash(
        &self,
        agent: AgentKind,
        project_dir: String,
        helper_sha256: String,
    ) -> Result<Vec<String>, SshCoreError> {
        let _ = agent_name(agent)?;
        let v = self
            .skills_call(
                &helper_sha256,
                "trash",
                agent,
                &project_dir,
                None,
                json!({}),
            )
            .await?;
        Ok(v.get("trash")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect())
    }
}
