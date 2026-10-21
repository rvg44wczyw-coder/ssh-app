use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
    /// Git diff (`HEAD`) of the repository the tmux session's pane is working in. The command runs
    /// in the pane's directory: an exec channel starts in `$HOME`, where `git diff` just prints
    /// its usage text.
    pub async fn fetch_git_diff(
        &self,
        session_name: String,
    ) -> Result<crate::gitdiff::GitDiffResult, SshCoreError> {
        let cmd = crate::gitdiff::build_git_diff_command(&session_name)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitdiff::parse_git_diff_output(&out))
    }

    /// Which files of the repository containing `dir` are new, changed or deleted (for the file
    /// browser). `is_repo` is false outside a repository.
    pub async fn fetch_git_status(
        &self,
        dir: String,
    ) -> Result<crate::gitstatus::GitStatusResult, SshCoreError> {
        let cmd = crate::gitstatus::build_git_status_command(&dir)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitstatus::parse_git_status_output(&out))
    }

    /// The latest commits of the repository the tab's pane is in (newest first).
    pub async fn fetch_git_log(
        &self,
        session_name: String,
        limit: u32,
    ) -> Result<crate::gitlog::GitLogResult, SshCoreError> {
        let cmd = crate::gitlog::build_git_log_command(&session_name, limit)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitlog::parse_git_log_output(&out, limit))
    }

    /// The patch of one commit, shaped like a diff result for the diff viewer.
    pub async fn fetch_git_show(
        &self,
        session_name: String,
        hash: String,
    ) -> Result<crate::gitdiff::GitDiffResult, SshCoreError> {
        let cmd = crate::gitlog::build_git_show_command(&session_name, &hash)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitlog::parse_git_show_output(&out))
    }

    /// Branches of the repository the tab's pane is in (feature 28, G1).
    pub async fn fetch_git_branches(
        &self,
        session_name: String,
    ) -> Result<crate::gitbranch::GitBranchesResult, SshCoreError> {
        let cmd = crate::gitbranch::build_git_branches_command(&session_name)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitbranch::parse_git_branches_output(&out))
    }

    /// Switches, creates or deletes a branch (see `GitBranchOp`). Invalid names are refused here
    /// as an error; git's own refusals come back in the result.
    pub async fn run_git_branch_op(
        &self,
        session_name: String,
        op: crate::gitbranch::GitBranchOp,
    ) -> Result<crate::gitops::GitOpResult, SshCoreError> {
        let cmd = crate::gitbranch::build_git_branch_op_command(&session_name, &op)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitbranch::parse_git_branch_op_output(&out))
    }

    /// Worktrees of the repository the tab's pane is in (feature 28, G2).
    pub async fn fetch_git_worktrees(
        &self,
        session_name: String,
    ) -> Result<crate::gitworktree::GitWorktreesResult, SshCoreError> {
        let cmd = crate::gitworktree::build_git_worktrees_command(&session_name)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitworktree::parse_git_worktrees_output(&out))
    }

    /// Adds or removes a worktree (see `GitWorktreeOp`).
    pub async fn run_git_worktree_op(
        &self,
        session_name: String,
        op: crate::gitworktree::GitWorktreeOp,
    ) -> Result<crate::gitops::GitOpResult, SshCoreError> {
        let cmd = crate::gitworktree::build_git_worktree_op_command(&session_name, &op)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitworktree::parse_git_worktree_op_output(&out))
    }

    /// Cherry-pick, revert, reset (soft / mixed) or reword the last commit (feature 28, G3).
    pub async fn run_git_commit_op(
        &self,
        session_name: String,
        op: crate::gitcommit::GitCommitOp,
    ) -> Result<crate::gitops::GitOpResult, SshCoreError> {
        let cmd = crate::gitcommit::build_git_commit_op_command(&session_name, &op)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitcommit::parse_git_commit_op_output(&out))
    }

    /// Squash, drop, merge, rebase and stash operations (feature 28, G4).
    pub async fn run_git_history_op(
        &self,
        session_name: String,
        op: crate::githistory::GitHistoryOp,
    ) -> Result<crate::gitops::GitOpResult, SshCoreError> {
        let cmd = crate::githistory::build_git_history_op_command(&session_name, &op)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::githistory::parse_git_history_op_output(&out))
    }

    /// The stash list of the repository the tab is in.
    pub async fn fetch_git_stashes(
        &self,
        session_name: String,
    ) -> Result<crate::githistory::GitStashesResult, SshCoreError> {
        let cmd = crate::githistory::build_git_stashes_command(&session_name)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::githistory::parse_git_stashes_output(&out))
    }

    /// Staged / unstaged changes with branch and upstream state (feature 28, G5).
    pub async fn fetch_git_changes(
        &self,
        session_name: String,
    ) -> Result<crate::gitindex::GitChangesResult, SshCoreError> {
        let cmd = crate::gitindex::build_git_changes_command(&session_name)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitindex::parse_git_changes_output(&out))
    }

    /// Stage, unstage, commit, push, pull (fast-forward only) or fetch (feature 28, G5).
    pub async fn run_git_index_op(
        &self,
        session_name: String,
        op: crate::gitindex::GitIndexOp,
    ) -> Result<crate::gitops::GitOpResult, SshCoreError> {
        let cmd = crate::gitindex::build_git_index_op_command(&session_name, &op)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitindex::parse_git_index_op_output(&out))
    }

    /// The hunks of one file's diff (`staged`: the index against HEAD, else the working tree
    /// against the index), for staging single hunks (feature 28, G5b).
    pub async fn fetch_git_file_hunks(
        &self,
        session_name: String,
        path: String,
        staged: bool,
    ) -> Result<crate::githunks::GitFileHunks, SshCoreError> {
        let cmd = crate::githunks::build_git_hunks_command(&session_name, &path, staged)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::githunks::parse_git_hunks_output(&out))
    }

    /// Puts the chosen hunks in the index (or takes them out again with `unstage`); the working
    /// tree is not touched.
    pub async fn apply_git_hunks(
        &self,
        session_name: String,
        path: String,
        hunks: Vec<crate::githunks::GitHunk>,
        unstage: bool,
    ) -> Result<crate::gitops::GitOpResult, SshCoreError> {
        let cmd =
            crate::githunks::build_git_apply_hunks_command(&session_name, &path, &hunks, unstage)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::githunks::parse_git_apply_output(&out))
    }
}
