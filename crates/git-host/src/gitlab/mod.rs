//! GitLab hosting service implementation.

mod cli;

use std::{path::Path, time::Duration};

use async_trait::async_trait;
use backon::{ExponentialBuilder, Retryable};
pub use cli::GlabCli;
use cli::{GitLabRepoInfo, GlabCliError};
use db::models::merge::PullRequestInfo;
use tokio::task;
use tracing::info;

use crate::{
    GitHostProvider,
    types::{CreatePrRequest, GitHostError, OpenPrInfo, ProviderKind, UnifiedPrComment},
};

#[derive(Debug, Clone)]
pub struct GitLabProvider {
    glab_cli: GlabCli,
}

impl GitLabProvider {
    pub fn new() -> Result<Self, GitHostError> {
        Ok(Self {
            glab_cli: GlabCli::new(),
        })
    }

    async fn get_repo_info(
        &self,
        remote_url: &str,
        repo_path: &Path,
    ) -> Result<GitLabRepoInfo, GitHostError> {
        let cli = self.glab_cli.clone();
        let url = remote_url.to_string();
        let path = repo_path.to_path_buf();
        task::spawn_blocking(move || cli.get_repo_info(&url, &path))
            .await
            .map_err(|err| {
                GitHostError::Repository(format!("Failed to get repo info from URL: {err}"))
            })?
            .map_err(Into::into)
    }
}

impl From<GlabCliError> for GitHostError {
    fn from(error: GlabCliError) -> Self {
        match &error {
            GlabCliError::AuthFailed(msg) => GitHostError::AuthFailed(msg.clone()),
            GlabCliError::NotAvailable => GitHostError::CliNotInstalled {
                provider: ProviderKind::GitLab,
            },
            GlabCliError::CommandFailed(msg) => {
                let lower = msg.to_ascii_lowercase();
                if lower.contains("403") || lower.contains("forbidden") {
                    GitHostError::InsufficientPermissions(msg.clone())
                } else if lower.contains("404") || lower.contains("not found") {
                    GitHostError::RepoNotFoundOrNoAccess(msg.clone())
                } else if lower.contains("not a git repository") {
                    GitHostError::NotAGitRepository(msg.clone())
                } else {
                    GitHostError::PullRequest(msg.clone())
                }
            }
            GlabCliError::UnexpectedOutput(msg) => GitHostError::UnexpectedOutput(msg.clone()),
        }
    }
}

#[async_trait]
impl GitHostProvider for GitLabProvider {
    async fn create_pr(
        &self,
        repo_path: &Path,
        remote_url: &str,
        request: &CreatePrRequest,
    ) -> Result<PullRequestInfo, GitHostError> {
        if let Some(head_url) = &request.head_repo_url
            && head_url != remote_url
        {
            return Err(GitHostError::PullRequest(
                "Cross-fork merge requests are not supported for GitLab".to_string(),
            ));
        }

        let repo_info = self.get_repo_info(remote_url, repo_path).await?;

        (|| async {
            let cli = self.glab_cli.clone();
            let request = request.clone();
            let repo_info = repo_info.clone();
            let repo_path = repo_path.to_path_buf();

            let cli_result =
                task::spawn_blocking(move || cli.create_mr(&request, &repo_info, &repo_path))
                    .await
                    .map_err(|err| {
                        GitHostError::PullRequest(format!(
                            "Failed to execute GitLab CLI for MR creation: {err}"
                        ))
                    })?
                    .map_err(GitHostError::from)?;

            info!(
                "Created GitLab MR !{} for branch {}",
                cli_result.number, request.head_branch
            );

            Ok(cli_result)
        })
        .retry(
            &ExponentialBuilder::default()
                .with_min_delay(Duration::from_secs(1))
                .with_max_delay(Duration::from_secs(30))
                .with_max_times(3)
                .with_jitter(),
        )
        .when(|e: &GitHostError| e.should_retry())
        .notify(|err: &GitHostError, dur: Duration| {
            tracing::warn!(
                "GitLab API call failed, retrying after {:.2}s: {}",
                dur.as_secs_f64(),
                err
            );
        })
        .await
    }

    async fn get_pr_status(&self, pr_url: &str) -> Result<PullRequestInfo, GitHostError> {
        let cli = self.glab_cli.clone();
        let url = pr_url.to_string();

        (|| async {
            let cli = cli.clone();
            let url = url.clone();
            let pr = task::spawn_blocking(move || cli.view_mr(&url))
                .await
                .map_err(|err| {
                    GitHostError::PullRequest(format!(
                        "Failed to execute GitLab CLI for viewing MR: {err}"
                    ))
                })?;
            pr.map_err(GitHostError::from)
        })
        .retry(
            &ExponentialBuilder::default()
                .with_min_delay(Duration::from_secs(1))
                .with_max_delay(Duration::from_secs(30))
                .with_max_times(3)
                .with_jitter(),
        )
        .when(|err: &GitHostError| err.should_retry())
        .notify(|err: &GitHostError, dur: Duration| {
            tracing::warn!(
                "GitLab API call failed, retrying after {:.2}s: {}",
                dur.as_secs_f64(),
                err
            );
        })
        .await
    }

    async fn list_prs_for_branch(
        &self,
        repo_path: &Path,
        remote_url: &str,
        branch_name: &str,
    ) -> Result<Vec<PullRequestInfo>, GitHostError> {
        let repo_info = self.get_repo_info(remote_url, repo_path).await?;

        let cli = self.glab_cli.clone();
        let branch = branch_name.to_string();

        (|| async {
            let cli = cli.clone();
            let repo_info = repo_info.clone();
            let branch = branch.clone();

            let prs = task::spawn_blocking(move || cli.list_mrs_for_branch(&repo_info, &branch))
                .await
                .map_err(|err| {
                    GitHostError::PullRequest(format!(
                        "Failed to execute GitLab CLI for listing MRs: {err}"
                    ))
                })?;
            prs.map_err(GitHostError::from)
        })
        .retry(
            &ExponentialBuilder::default()
                .with_min_delay(Duration::from_secs(1))
                .with_max_delay(Duration::from_secs(30))
                .with_max_times(3)
                .with_jitter(),
        )
        .when(|e: &GitHostError| e.should_retry())
        .notify(|err: &GitHostError, dur: Duration| {
            tracing::warn!(
                "GitLab API call failed, retrying after {:.2}s: {}",
                dur.as_secs_f64(),
                err
            );
        })
        .await
    }

    async fn get_pr_comments(
        &self,
        repo_path: &Path,
        remote_url: &str,
        pr_number: i64,
    ) -> Result<Vec<UnifiedPrComment>, GitHostError> {
        let repo_info = self.get_repo_info(remote_url, repo_path).await?;

        let cli = self.glab_cli.clone();

        (|| async {
            let cli = cli.clone();
            let repo_info = repo_info.clone();

            let comments = task::spawn_blocking(move || cli.get_mr_notes(&repo_info, pr_number))
                .await
                .map_err(|err| {
                    GitHostError::PullRequest(format!(
                        "Failed to execute GitLab CLI for fetching MR notes: {err}"
                    ))
                })?;
            comments.map_err(GitHostError::from)
        })
        .retry(
            &ExponentialBuilder::default()
                .with_min_delay(Duration::from_secs(1))
                .with_max_delay(Duration::from_secs(30))
                .with_max_times(3)
                .with_jitter(),
        )
        .when(|e: &GitHostError| e.should_retry())
        .notify(|err: &GitHostError, dur: Duration| {
            tracing::warn!(
                "GitLab API call failed, retrying after {:.2}s: {}",
                dur.as_secs_f64(),
                err
            );
        })
        .await
    }

    async fn list_open_prs(
        &self,
        repo_path: &Path,
        remote_url: &str,
    ) -> Result<Vec<OpenPrInfo>, GitHostError> {
        let repo_info = self.get_repo_info(remote_url, repo_path).await?;

        let cli = self.glab_cli.clone();

        (|| async {
            let cli = cli.clone();
            let repo_info = repo_info.clone();

            let prs = task::spawn_blocking(move || cli.list_open_mrs(&repo_info))
                .await
                .map_err(|err| {
                    GitHostError::PullRequest(format!(
                        "Failed to execute GitLab CLI for listing open MRs: {err}"
                    ))
                })?;
            prs.map_err(GitHostError::from)
        })
        .retry(
            &ExponentialBuilder::default()
                .with_min_delay(Duration::from_secs(1))
                .with_max_delay(Duration::from_secs(30))
                .with_max_times(3)
                .with_jitter(),
        )
        .when(|e: &GitHostError| e.should_retry())
        .notify(|err: &GitHostError, dur: Duration| {
            tracing::warn!(
                "GitLab API call failed, retrying after {:.2}s: {}",
                dur.as_secs_f64(),
                err
            );
        })
        .await
    }

    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::GitLab
    }
}
