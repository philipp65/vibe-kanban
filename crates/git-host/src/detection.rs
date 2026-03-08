//! Git hosting provider detection from repository URLs.

use crate::types::ProviderKind;

/// Detect the git hosting provider from a remote URL.
///
/// Supports:
/// - GitHub.com: `https://github.com/owner/repo` or `git@github.com:owner/repo.git`
/// - GitHub Enterprise: URLs containing `github.` (e.g., `https://github.company.com/owner/repo`)
/// - Azure DevOps: `https://dev.azure.com/org/project/_git/repo` or legacy `https://org.visualstudio.com/...`
/// - GitLab.com: `https://gitlab.com/owner/repo` or `git@gitlab.com:owner/repo.git`
pub fn detect_provider_from_url(url: &str) -> ProviderKind {
    detect_provider_from_url_with_custom_gitlab_domains(url, &[])
}

/// Detect the git hosting provider from a remote URL, with additional custom
/// GitLab hostnames for self-hosted instances.
pub fn detect_provider_from_url_with_custom_gitlab_domains(
    url: &str,
    custom_gitlab_domains: &[String],
) -> ProviderKind {
    let url_lower = url.to_lowercase();

    if url_lower.contains("github.com") {
        return ProviderKind::GitHub;
    }

    // Check Azure patterns before GHE to avoid false positives
    if url_lower.contains("dev.azure.com")
        || url_lower.contains(".visualstudio.com")
        || url_lower.contains("ssh.dev.azure.com")
    {
        return ProviderKind::AzureDevOps;
    }

    // /_git/ is unique to Azure DevOps
    if url_lower.contains("/_git/") {
        return ProviderKind::AzureDevOps;
    }

    if url_lower.contains("gitlab.com") {
        return ProviderKind::GitLab;
    }

    // Check custom self-hosted GitLab domains
    for domain in custom_gitlab_domains {
        let domain_lower = domain.to_lowercase();
        // Strip protocol prefix if present in the configured domain
        let hostname = domain_lower
            .strip_prefix("https://")
            .or_else(|| domain_lower.strip_prefix("http://"))
            .unwrap_or(&domain_lower);
        if url_lower.contains(hostname) {
            return ProviderKind::GitLab;
        }
    }

    // GitHub Enterprise (contains "github." but not the Azure patterns above)
    if url_lower.contains("github.") {
        return ProviderKind::GitHub;
    }

    ProviderKind::Unknown
}

/// Detect the git hosting provider from a PR/MR URL.
///
/// Supports:
/// - GitHub: `https://github.com/owner/repo/pull/123`
/// - GitHub Enterprise: `https://github.company.com/owner/repo/pull/123`
/// - Azure DevOps: `https://dev.azure.com/org/project/_git/repo/pullrequest/123`
/// - GitLab: `https://gitlab.com/owner/repo/-/merge_requests/123`
#[cfg(test)]
fn detect_provider_from_pr_url(pr_url: &str) -> ProviderKind {
    detect_provider_from_pr_url_with_custom_gitlab_domains(pr_url, &[])
}

#[cfg(test)]
fn detect_provider_from_pr_url_with_custom_gitlab_domains(
    pr_url: &str,
    custom_gitlab_domains: &[String],
) -> ProviderKind {
    let url_lower = pr_url.to_lowercase();

    // GitHub pattern: contains /pull/ in the path
    if url_lower.contains("/pull/") {
        if url_lower.contains("github.com") || url_lower.contains("github.") {
            return ProviderKind::GitHub;
        }
    }

    // Azure DevOps pattern: contains /pullrequest/ in the path
    if url_lower.contains("/pullrequest/") {
        return ProviderKind::AzureDevOps;
    }

    // GitLab pattern: contains /-/merge_requests/ in the path
    if url_lower.contains("/-/merge_requests/") {
        return ProviderKind::GitLab;
    }

    // Fall back to general URL detection
    detect_provider_from_url_with_custom_gitlab_domains(pr_url, custom_gitlab_domains)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_github_com_https() {
        assert_eq!(
            detect_provider_from_url("https://github.com/owner/repo"),
            ProviderKind::GitHub
        );
        assert_eq!(
            detect_provider_from_url("https://github.com/owner/repo.git"),
            ProviderKind::GitHub
        );
    }

    #[test]
    fn test_github_com_ssh() {
        assert_eq!(
            detect_provider_from_url("git@github.com:owner/repo.git"),
            ProviderKind::GitHub
        );
    }

    #[test]
    fn test_github_enterprise() {
        assert_eq!(
            detect_provider_from_url("https://github.company.com/owner/repo"),
            ProviderKind::GitHub
        );
        assert_eq!(
            detect_provider_from_url("https://github.acme.corp/team/project"),
            ProviderKind::GitHub
        );
        assert_eq!(
            detect_provider_from_url("git@github.internal.io:org/repo.git"),
            ProviderKind::GitHub
        );
    }

    #[test]
    fn test_azure_devops_https() {
        assert_eq!(
            detect_provider_from_url("https://dev.azure.com/org/project/_git/repo"),
            ProviderKind::AzureDevOps
        );
    }

    #[test]
    fn test_azure_devops_ssh() {
        assert_eq!(
            detect_provider_from_url("git@ssh.dev.azure.com:v3/org/project/repo"),
            ProviderKind::AzureDevOps
        );
    }

    #[test]
    fn test_azure_devops_legacy_visualstudio() {
        assert_eq!(
            detect_provider_from_url("https://org.visualstudio.com/project/_git/repo"),
            ProviderKind::AzureDevOps
        );
    }

    #[test]
    fn test_azure_devops_git_path() {
        assert_eq!(
            detect_provider_from_url("https://custom.domain.com/org/project/_git/repo"),
            ProviderKind::AzureDevOps
        );
    }

    #[test]
    fn test_gitlab_com_https() {
        assert_eq!(
            detect_provider_from_url("https://gitlab.com/owner/repo"),
            ProviderKind::GitLab
        );
        assert_eq!(
            detect_provider_from_url("https://gitlab.com/owner/repo.git"),
            ProviderKind::GitLab
        );
    }

    #[test]
    fn test_gitlab_com_ssh() {
        assert_eq!(
            detect_provider_from_url("git@gitlab.com:owner/repo.git"),
            ProviderKind::GitLab
        );
    }

    #[test]
    fn test_gitlab_self_hosted_with_custom_domain() {
        let domains = vec!["gitlab.mycompany.com".to_string()];
        assert_eq!(
            detect_provider_from_url_with_custom_gitlab_domains(
                "https://gitlab.mycompany.com/team/project",
                &domains
            ),
            ProviderKind::GitLab
        );
        assert_eq!(
            detect_provider_from_url_with_custom_gitlab_domains(
                "git@gitlab.mycompany.com:team/project.git",
                &domains
            ),
            ProviderKind::GitLab
        );
    }

    #[test]
    fn test_gitlab_self_hosted_with_url_prefix() {
        let domains = vec!["https://gitlab.mycompany.com".to_string()];
        assert_eq!(
            detect_provider_from_url_with_custom_gitlab_domains(
                "https://gitlab.mycompany.com/team/project",
                &domains
            ),
            ProviderKind::GitLab
        );
    }

    #[test]
    fn test_unknown_provider() {
        assert_eq!(
            detect_provider_from_url("https://bitbucket.org/owner/repo"),
            ProviderKind::Unknown
        );
        assert_eq!(
            detect_provider_from_url("https://custom-git.example.com/owner/repo"),
            ProviderKind::Unknown
        );
    }

    #[test]
    fn test_pr_url_github() {
        assert_eq!(
            detect_provider_from_pr_url("https://github.com/owner/repo/pull/123"),
            ProviderKind::GitHub
        );
        assert_eq!(
            detect_provider_from_pr_url("https://github.company.com/owner/repo/pull/456"),
            ProviderKind::GitHub
        );
    }

    #[test]
    fn test_pr_url_azure() {
        assert_eq!(
            detect_provider_from_pr_url(
                "https://dev.azure.com/org/project/_git/repo/pullrequest/123"
            ),
            ProviderKind::AzureDevOps
        );
        assert_eq!(
            detect_provider_from_pr_url(
                "https://org.visualstudio.com/project/_git/repo/pullrequest/456"
            ),
            ProviderKind::AzureDevOps
        );
    }

    #[test]
    fn test_pr_url_gitlab() {
        assert_eq!(
            detect_provider_from_pr_url("https://gitlab.com/owner/repo/-/merge_requests/123"),
            ProviderKind::GitLab
        );
    }

    #[test]
    fn test_mr_url_gitlab_self_hosted() {
        let domains = vec!["gitlab.mycompany.com".to_string()];
        assert_eq!(
            detect_provider_from_pr_url_with_custom_gitlab_domains(
                "https://gitlab.mycompany.com/team/project/-/merge_requests/42",
                &domains
            ),
            ProviderKind::GitLab
        );
    }
}
