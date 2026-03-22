//! Minimal helpers around the GitLab CLI (`glab`).
//!
//! This module provides low-level access to the GitLab CLI for merge request
//! and repository operations on both gitlab.com and self-hosted instances.

use std::{
    ffi::{OsStr, OsString},
    io::Write,
    path::Path,
    process::Command,
};

use chrono::{DateTime, Utc};
use db::models::merge::{MergeStatus, PullRequestInfo};
use serde::Deserialize;
use tempfile::NamedTempFile;
use thiserror::Error;
use url::Url;
use utils::shell::resolve_executable_path_blocking;

use crate::types::{CreatePrRequest, OpenPrInfo, UnifiedPrComment};

#[derive(Debug, Clone)]
pub struct GitLabRepoInfo {
    pub project_path: String,
    pub hostname: Option<String>,
}

impl GitLabRepoInfo {
    pub fn repo_spec(&self) -> String {
        self.project_path.clone()
    }
}

#[derive(Deserialize)]
struct GlabMrCreateResponse {
    iid: i64,
    web_url: String,
}

#[derive(Deserialize)]
struct GlabMrViewResponse {
    iid: i64,
    web_url: String,
    state: String,
    merged_at: Option<String>,
    merge_commit_sha: Option<String>,
}

#[derive(Deserialize)]
struct GlabMrListResponse {
    iid: i64,
    web_url: String,
    state: String,
    merged_at: Option<String>,
    merge_commit_sha: Option<String>,
}

#[derive(Deserialize)]
struct GlabMrListExtendedResponse {
    iid: i64,
    web_url: String,
    title: String,
    source_branch: String,
    target_branch: String,
}

#[derive(Deserialize)]
struct GlabNoteResponse {
    id: i64,
    body: String,
    author: GlabNoteAuthor,
    created_at: String,
    #[serde(rename = "type")]
    note_type: Option<String>,
    #[serde(default)]
    system: bool,
    position: Option<GlabNotePosition>,
}

#[derive(Deserialize)]
struct GlabNoteAuthor {
    username: String,
}

#[derive(Deserialize)]
struct GlabNotePosition {
    new_path: Option<String>,
    new_line: Option<i64>,
}

#[derive(Debug, Error)]
pub enum GlabCliError {
    #[error("GitLab CLI (`glab`) executable not found or not runnable")]
    NotAvailable,
    #[error("GitLab CLI command failed: {0}")]
    CommandFailed(String),
    #[error("GitLab CLI authentication failed: {0}")]
    AuthFailed(String),
    #[error("GitLab CLI returned unexpected output: {0}")]
    UnexpectedOutput(String),
}

#[derive(Debug, Clone, Default)]
pub struct GlabCli;

impl GlabCli {
    pub fn new() -> Self {
        Self {}
    }

    fn ensure_available(&self) -> Result<(), GlabCliError> {
        resolve_executable_path_blocking("glab").ok_or(GlabCliError::NotAvailable)?;
        Ok(())
    }

    fn run<I, S>(&self, args: I, dir: Option<&Path>) -> Result<String, GlabCliError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.ensure_available()?;
        let glab = resolve_executable_path_blocking("glab").ok_or(GlabCliError::NotAvailable)?;
        let mut cmd = Command::new(&glab);
        if let Some(d) = dir {
            cmd.current_dir(d);
        }
        for arg in args {
            cmd.arg(arg);
        }

        let output = cmd
            .output()
            .map_err(|err| GlabCliError::CommandFailed(err.to_string()))?;

        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).to_string());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        let lower = stderr.to_ascii_lowercase();
        if lower.contains("glab auth login")
            || lower.contains("not logged in")
            || lower.contains("authentication")
            || lower.contains("unauthorized")
            || lower.contains("401")
        {
            return Err(GlabCliError::AuthFailed(stderr));
        }

        Err(GlabCliError::CommandFailed(stderr))
    }

    /// Extract project path (owner/repo) and hostname from a remote URL.
    pub fn get_repo_info(
        &self,
        remote_url: &str,
        _repo_path: &Path,
    ) -> Result<GitLabRepoInfo, GlabCliError> {
        Self::parse_repo_info_from_url(remote_url)
    }

    fn parse_repo_info_from_url(remote_url: &str) -> Result<GitLabRepoInfo, GlabCliError> {
        // SSH format: git@gitlab.com:owner/repo.git
        if let Some(rest) = remote_url.strip_prefix("git@") {
            if let Some((host, path)) = rest.split_once(':') {
                let project_path = path.trim_end_matches(".git").to_string();
                return Ok(GitLabRepoInfo {
                    project_path,
                    hostname: Some(host.to_string()),
                });
            }
        }

        // HTTPS format: https://gitlab.com/owner/repo.git
        if let Ok(parsed) = Url::parse(remote_url) {
            let hostname = parsed.host_str().map(String::from);
            let path = parsed
                .path()
                .trim_start_matches('/')
                .trim_end_matches(".git")
                .to_string();
            if !path.is_empty() {
                return Ok(GitLabRepoInfo {
                    project_path: path,
                    hostname,
                });
            }
        }

        Err(GlabCliError::UnexpectedOutput(format!(
            "Could not parse GitLab project from URL: {remote_url}"
        )))
    }

    pub fn create_mr(
        &self,
        request: &CreatePrRequest,
        repo_info: &GitLabRepoInfo,
        repo_path: &Path,
    ) -> Result<PullRequestInfo, GlabCliError> {
        let body = request.body.as_deref().unwrap_or("");
        let mut body_file = NamedTempFile::new()
            .map_err(|e| GlabCliError::CommandFailed(format!("Failed to create temp file: {e}")))?;
        body_file
            .write_all(body.as_bytes())
            .map_err(|e| GlabCliError::CommandFailed(format!("Failed to write body: {e}")))?;

        let repo_spec = repo_info.repo_spec();

        let mut args: Vec<OsString> = Vec::with_capacity(16);
        args.push(OsString::from("mr"));
        args.push(OsString::from("create"));
        args.push(OsString::from("--repo"));
        args.push(OsString::from(&repo_spec));
        args.push(OsString::from("--source-branch"));
        args.push(OsString::from(&request.head_branch));
        args.push(OsString::from("--target-branch"));
        args.push(OsString::from(&request.base_branch));
        args.push(OsString::from("--title"));
        args.push(OsString::from(&request.title));
        args.push(OsString::from("--description"));
        args.push(OsString::from(body));
        args.push(OsString::from("--no-editor"));
        args.push(OsString::from("--output"));
        args.push(OsString::from("json"));

        if request.draft.unwrap_or(false) {
            args.push(OsString::from("--draft"));
        }

        let raw = self.run(args, Some(repo_path))?;
        Self::parse_mr_create_response(&raw)
    }

    pub fn view_mr(&self, mr_url: &str) -> Result<PullRequestInfo, GlabCliError> {
        let raw = self.run(["mr", "view", mr_url, "--output", "json"], None)?;
        Self::parse_mr_view_response(&raw)
    }

    pub fn list_mrs_for_branch(
        &self,
        repo_info: &GitLabRepoInfo,
        branch: &str,
    ) -> Result<Vec<PullRequestInfo>, GlabCliError> {
        let repo_spec = repo_info.repo_spec();
        let raw = self.run(
            [
                "mr",
                "list",
                "--repo",
                &repo_spec,
                "--source-branch",
                branch,
                "--all",
                "--output",
                "json",
            ],
            None,
        )?;
        Self::parse_mr_list_response(&raw)
    }

    pub fn list_open_mrs(
        &self,
        repo_info: &GitLabRepoInfo,
    ) -> Result<Vec<OpenPrInfo>, GlabCliError> {
        let repo_spec = repo_info.repo_spec();
        let raw = self.run(
            [
                "mr", "list", "--repo", &repo_spec, "--state", "opened", "--output", "json",
            ],
            None,
        )?;
        Self::parse_open_mr_list(&raw)
    }

    pub fn get_mr_notes(
        &self,
        repo_info: &GitLabRepoInfo,
        mr_iid: i64,
    ) -> Result<Vec<UnifiedPrComment>, GlabCliError> {
        let repo_spec = repo_info.repo_spec();
        let raw = self.run(
            [
                "api",
                &format!(
                    "projects/{}/merge_requests/{}/notes?sort=asc&per_page=100",
                    urlencoding::encode(&repo_spec),
                    mr_iid,
                ),
            ],
            None,
        )?;
        Self::parse_mr_notes(&raw)
    }
}

impl GlabCli {
    fn parse_mr_create_response(raw: &str) -> Result<PullRequestInfo, GlabCliError> {
        // glab mr create --output json returns a JSON object
        if let Ok(mr) = serde_json::from_str::<GlabMrCreateResponse>(raw.trim()) {
            return Ok(PullRequestInfo {
                number: mr.iid,
                url: mr.web_url,
                status: MergeStatus::Open,
                merged_at: None,
                merge_commit_sha: None,
            });
        }

        // Fallback: parse plain text output if JSON output is not available.
        if let Some(info) = Self::parse_mr_create_text(raw) {
            return Ok(info);
        }

        Err(GlabCliError::UnexpectedOutput(format!(
            "Failed to parse glab mr create response; raw: {raw}"
        )))
    }

    fn parse_mr_create_text(raw: &str) -> Option<PullRequestInfo> {
        let mr_url = raw
            .lines()
            .rev()
            .flat_map(|line| line.split_whitespace())
            .map(|token| token.trim_matches(|c: char| c == '<' || c == '>'))
            .find(|token| token.starts_with("http") && token.contains("/-/merge_requests/"))?
            .trim_end_matches(['.', ',', ';'])
            .to_string();

        let number = Self::extract_mr_number_from_url(&mr_url)?;

        Some(PullRequestInfo {
            number,
            url: mr_url,
            status: MergeStatus::Open,
            merged_at: None,
            merge_commit_sha: None,
        })
    }

    fn extract_mr_number_from_url(url: &str) -> Option<i64> {
        url.rsplit('/').next().and_then(|s| {
            s.trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse::<i64>()
                .ok()
        })
    }

    fn parse_mr_view_response(raw: &str) -> Result<PullRequestInfo, GlabCliError> {
        let mr: GlabMrViewResponse = serde_json::from_str(raw.trim()).map_err(|e| {
            GlabCliError::UnexpectedOutput(format!(
                "Failed to parse glab mr view response: {e}; raw: {raw}"
            ))
        })?;

        let merged_at = mr
            .merged_at
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        Ok(PullRequestInfo {
            number: mr.iid,
            url: mr.web_url,
            status: Self::map_gitlab_state(&mr.state),
            merged_at,
            merge_commit_sha: mr.merge_commit_sha,
        })
    }

    fn parse_mr_list_response(raw: &str) -> Result<Vec<PullRequestInfo>, GlabCliError> {
        let mrs: Vec<GlabMrListResponse> = serde_json::from_str(raw.trim()).map_err(|e| {
            GlabCliError::UnexpectedOutput(format!(
                "Failed to parse glab mr list response: {e}; raw: {raw}"
            ))
        })?;

        Ok(mrs
            .into_iter()
            .map(|mr| {
                let merged_at = mr
                    .merged_at
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                PullRequestInfo {
                    number: mr.iid,
                    url: mr.web_url,
                    status: Self::map_gitlab_state(&mr.state),
                    merged_at,
                    merge_commit_sha: mr.merge_commit_sha,
                }
            })
            .collect())
    }

    fn parse_open_mr_list(raw: &str) -> Result<Vec<OpenPrInfo>, GlabCliError> {
        let mrs: Vec<GlabMrListExtendedResponse> =
            serde_json::from_str(raw.trim()).map_err(|e| {
                GlabCliError::UnexpectedOutput(format!(
                    "Failed to parse glab mr list response: {e}; raw: {raw}"
                ))
            })?;
        Ok(mrs
            .into_iter()
            .map(|mr| OpenPrInfo {
                number: mr.iid,
                url: mr.web_url,
                title: mr.title,
                head_branch: mr.source_branch,
                base_branch: mr.target_branch,
            })
            .collect())
    }

    fn parse_mr_notes(raw: &str) -> Result<Vec<UnifiedPrComment>, GlabCliError> {
        let notes: Vec<GlabNoteResponse> = serde_json::from_str(raw.trim()).map_err(|e| {
            GlabCliError::UnexpectedOutput(format!(
                "Failed to parse MR notes response: {e}; raw: {raw}"
            ))
        })?;

        let mut comments: Vec<UnifiedPrComment> = Vec::new();

        for note in notes {
            if note.system {
                continue;
            }

            let created_at = DateTime::parse_from_rfc3339(&note.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let is_diff_note = note.note_type.as_deref() == Some("DiffNote");

            if is_diff_note {
                let path = note
                    .position
                    .as_ref()
                    .and_then(|p| p.new_path.clone())
                    .unwrap_or_default();
                let line = note.position.as_ref().and_then(|p| p.new_line);

                comments.push(UnifiedPrComment::Review {
                    id: note.id,
                    author: note.author.username,
                    author_association: None,
                    body: note.body,
                    created_at,
                    url: None,
                    path,
                    line,
                    side: None,
                    diff_hunk: None,
                });
            } else {
                comments.push(UnifiedPrComment::General {
                    id: note.id.to_string(),
                    author: note.author.username,
                    author_association: None,
                    body: note.body,
                    created_at,
                    url: None,
                });
            }
        }

        Ok(comments)
    }

    fn map_gitlab_state(state: &str) -> MergeStatus {
        match state.to_lowercase().as_str() {
            "opened" => MergeStatus::Open,
            "merged" => MergeStatus::Merged,
            "closed" => MergeStatus::Closed,
            _ => MergeStatus::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repo_info_https() {
        let info = GlabCli::parse_repo_info_from_url("https://gitlab.com/owner/repo.git").unwrap();
        assert_eq!(info.project_path, "owner/repo");
        assert_eq!(info.hostname.as_deref(), Some("gitlab.com"));
    }

    #[test]
    fn test_parse_repo_info_ssh() {
        let info = GlabCli::parse_repo_info_from_url("git@gitlab.com:owner/repo.git").unwrap();
        assert_eq!(info.project_path, "owner/repo");
        assert_eq!(info.hostname.as_deref(), Some("gitlab.com"));
    }

    #[test]
    fn test_parse_repo_info_subgroup() {
        let info =
            GlabCli::parse_repo_info_from_url("https://gitlab.com/group/subgroup/repo").unwrap();
        assert_eq!(info.project_path, "group/subgroup/repo");
    }

    #[test]
    fn test_parse_repo_info_self_hosted() {
        let info =
            GlabCli::parse_repo_info_from_url("https://gitlab.mycompany.com/team/project.git")
                .unwrap();
        assert_eq!(info.project_path, "team/project");
        assert_eq!(info.hostname.as_deref(), Some("gitlab.mycompany.com"));
    }

    #[test]
    fn test_extract_mr_number_from_url() {
        assert_eq!(
            GlabCli::extract_mr_number_from_url(
                "https://gitlab.com/owner/repo/-/merge_requests/42"
            ),
            Some(42)
        );
    }

    #[test]
    fn test_map_gitlab_state() {
        assert!(matches!(
            GlabCli::map_gitlab_state("opened"),
            MergeStatus::Open
        ));
        assert!(matches!(
            GlabCli::map_gitlab_state("merged"),
            MergeStatus::Merged
        ));
        assert!(matches!(
            GlabCli::map_gitlab_state("closed"),
            MergeStatus::Closed
        ));
        assert!(matches!(
            GlabCli::map_gitlab_state("something"),
            MergeStatus::Unknown
        ));
    }
}
