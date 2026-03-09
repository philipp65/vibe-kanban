use api_types::{BulkMigrateRequest, MigrateIssueRequest, MigrateProjectRequest};
use axum::{
    Json, Router,
    extract::{Extension, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;
use urlencoding::encode;
use uuid::Uuid;

use crate::{
    AppState,
    auth::RequestContext,
    db::{migration::MigrationRepository, oauth_accounts::OAuthAccountRepository},
    routes::{
        error::ErrorResponse,
        organization_members::{ensure_member_access, ensure_project_access},
    },
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/gitlab/import/project-issues", post(import_project_issues))
        .route("/gitlab/projects/search", get(search_projects))
}

#[derive(Debug, Deserialize)]
pub struct ImportGitLabProjectIssuesRequest {
    pub organization_id: Uuid,
    /// GitLab path, e.g. "group/subgroup/project"
    pub gitlab_project_path: String,
    #[serde(default)]
    pub include_closed_issues: bool,
}

#[derive(Debug, Serialize)]
pub struct ImportGitLabProjectIssuesResponse {
    pub project_id: Uuid,
    pub project_name: String,
    pub imported_issues: usize,
}

#[derive(Debug, Deserialize)]
struct GitLabProject {
    id: i64,
    name: String,
    web_url: Option<String>,
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitLabIssue {
    iid: i64,
    title: String,
    description: Option<String>,
    state: String,
    web_url: Option<String>,
    created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchGitLabProjectsRequest {
    #[serde(default)]
    pub query: String,
}

#[derive(Debug, Serialize)]
pub struct SearchGitLabProjectsResponse {
    pub projects: Vec<GitLabProjectSearchResult>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GitLabProjectSearchResult {
    pub id: i64,
    pub name: String,
    pub path_with_namespace: String,
    pub name_with_namespace: Option<String>,
    pub web_url: Option<String>,
}

async fn get_gitlab_access_token(
    state: &AppState,
    user_id: Uuid,
) -> Result<String, ErrorResponse> {
    let account_repo = OAuthAccountRepository::new(state.pool());
    let account = account_repo
        .get_by_user_provider(user_id, "gitlab")
        .await
        .map_err(|error| {
            tracing::error!(?error, user_id = %user_id, "failed to fetch gitlab oauth account");
            ErrorResponse::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to load GitLab account",
            )
        })?
        .ok_or_else(|| {
            ErrorResponse::new(
                StatusCode::BAD_REQUEST,
                "GitLab account is not linked. Please sign in with GitLab first.",
            )
        })?;

    let encrypted = account.encrypted_provider_tokens.ok_or_else(|| {
        ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "GitLab account token is missing. Please re-authenticate.",
        )
    })?;

    let details = state
        .jwt()
        .decrypt_provider_tokens(&encrypted)
        .map_err(|error| {
            tracing::error!(?error, user_id = %user_id, "failed to decrypt gitlab provider token");
            ErrorResponse::new(
                StatusCode::BAD_REQUEST,
                "Invalid GitLab token state. Please sign in again.",
            )
        })?;

    Ok(details.access_token)
}

fn gitlab_base_url(state: &AppState) -> String {
    state
        .config
        .auth
        .gitlab()
        .and_then(|cfg| cfg.base_url.clone())
        .unwrap_or_else(|| "https://gitlab.com".to_string())
        .trim_end_matches('/')
        .to_string()
}

fn parse_project_path_from_reference(base_url: &str, reference: &str) -> String {
    let trimmed = reference.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }

    let parsed_base = Url::parse(base_url).ok();
    let parsed_ref = Url::parse(trimmed).ok();

    if let (Some(base), Some(reference_url)) = (parsed_base, parsed_ref) {
        if base.host_str() == reference_url.host_str() {
            let mut parts = reference_url
                .path_segments()
                .map(|segments| {
                    segments
                        .filter(|segment| !segment.is_empty())
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            if let Some(suffix_start) = parts.iter().position(|segment| segment == "-") {
                parts.truncate(suffix_start);
            }

            if !parts.is_empty() {
                return parts.join("/");
            }
        }
    }

    trimmed.to_string()
}

async fn build_gitlab_error_response(
    response: reqwest::Response,
    action: &str,
) -> ErrorResponse {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let body_msg = body
        .trim()
        .chars()
        .take(300)
        .collect::<String>()
        .replace('\n', " ");

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return ErrorResponse::new(
            StatusCode::FORBIDDEN,
            format!(
                "GitLab authorization failed while {action}. Reconnect GitLab with scope read_api (and read_user/openid/email)."
            ),
        );
    }

    if status == StatusCode::NOT_FOUND {
        return ErrorResponse::new(
            StatusCode::NOT_FOUND,
            "GitLab project not found or not accessible",
        );
    }

    let suffix = if body_msg.is_empty() {
        String::new()
    } else {
        format!(": {body_msg}")
    };

    ErrorResponse::new(
        StatusCode::BAD_GATEWAY,
        format!("GitLab API error while {action} (status {status}){suffix}"),
    )
}

async fn fetch_gitlab_project(
    state: &AppState,
    access_token: &str,
    base_url: &str,
    project_path: &str,
) -> Result<GitLabProject, ErrorResponse> {
    let project_path_encoded = encode(project_path);
    let url = format!("{base_url}/api/v4/projects/{project_path_encoded}");

    let response = state
        .http_client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| {
            tracing::error!(?error, "failed to call GitLab project endpoint");
            ErrorResponse::new(StatusCode::BAD_GATEWAY, "failed to reach GitLab API")
        })?;

    if !response.status().is_success() {
        tracing::warn!(
            status = %response.status(),
            "gitlab project endpoint returned non-success"
        );
        return Err(build_gitlab_error_response(response, "loading project").await);
    }

    response.json::<GitLabProject>().await.map_err(|error| {
        tracing::error!(?error, "failed to decode GitLab project response");
        ErrorResponse::new(
            StatusCode::BAD_GATEWAY,
            "Invalid GitLab response while loading project",
        )
    })
}

async fn fetch_gitlab_issues(
    state: &AppState,
    access_token: &str,
    base_url: &str,
    project_id: i64,
    include_closed: bool,
) -> Result<Vec<GitLabIssue>, ErrorResponse> {
    let mut page = 1;
    let state_filter = if include_closed { "all" } else { "opened" };
    let mut all_issues = Vec::new();

    loop {
        let url = format!(
            "{base_url}/api/v4/projects/{project_id}/issues?state={state_filter}&per_page=100&page={page}"
        );
        let response = state
            .http_client
            .get(url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|error| {
                tracing::error!(?error, project_id, page, "failed to call GitLab issues endpoint");
                ErrorResponse::new(StatusCode::BAD_GATEWAY, "failed to reach GitLab API")
            })?;

        if !response.status().is_success() {
            tracing::warn!(
                status = %response.status(),
                project_id,
                page,
                "gitlab issues endpoint returned non-success"
            );
            return Err(build_gitlab_error_response(response, "loading issues").await);
        }

        let next_page = response
            .headers()
            .get("x-next-page")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0);

        let mut issues_page = response.json::<Vec<GitLabIssue>>().await.map_err(|error| {
            tracing::error!(?error, project_id, page, "failed to decode GitLab issues response");
            ErrorResponse::new(
                StatusCode::BAD_GATEWAY,
                "Invalid GitLab response while loading issues",
            )
        })?;

        all_issues.append(&mut issues_page);

        if next_page == 0 {
            break;
        }
        page = next_page;
    }

    Ok(all_issues)
}

async fn fetch_gitlab_projects(
    state: &AppState,
    access_token: &str,
    base_url: &str,
    query: &str,
) -> Result<Vec<GitLabProjectSearchResult>, ErrorResponse> {
    let query_trimmed = query.trim();
    let search_param = if query_trimmed.is_empty() {
        String::new()
    } else {
        format!("&search={}", encode(query_trimmed))
    };
    let url = format!(
        "{base_url}/api/v4/projects?membership=true&simple=true&per_page=100&order_by=last_activity_at&sort=desc{search_param}"
    );

    let response = state
        .http_client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| {
            tracing::error!(?error, "failed to call GitLab projects endpoint");
            ErrorResponse::new(StatusCode::BAD_GATEWAY, "failed to reach GitLab API")
        })?;

    if !response.status().is_success() {
        tracing::warn!(
            status = %response.status(),
            "gitlab projects endpoint returned non-success"
        );
        return Err(build_gitlab_error_response(response, "searching projects").await);
    }

    response
        .json::<Vec<GitLabProjectSearchResult>>()
        .await
        .map_err(|error| {
            tracing::error!(?error, "failed to decode GitLab projects response");
            ErrorResponse::new(
                StatusCode::BAD_GATEWAY,
                "Invalid GitLab response while searching projects",
            )
        })
}

pub async fn search_projects(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Query(payload): Query<SearchGitLabProjectsRequest>,
) -> Result<Json<SearchGitLabProjectsResponse>, ErrorResponse> {
    let access_token = get_gitlab_access_token(&state, ctx.user.id).await?;
    let base_url = gitlab_base_url(&state);

    let projects = fetch_gitlab_projects(&state, &access_token, &base_url, &payload.query).await?;

    Ok(Json(SearchGitLabProjectsResponse { projects }))
}

pub async fn import_project_issues(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(payload): Json<ImportGitLabProjectIssuesRequest>,
) -> Result<Json<ImportGitLabProjectIssuesResponse>, ErrorResponse> {
    ensure_member_access(state.pool(), payload.organization_id, ctx.user.id).await?;

    let base_url = gitlab_base_url(&state);
    let project_path = parse_project_path_from_reference(&base_url, &payload.gitlab_project_path);
    if project_path.is_empty() {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "GitLab project path is required",
        ));
    }

    let access_token = get_gitlab_access_token(&state, ctx.user.id).await?;

    let gitlab_project =
        fetch_gitlab_project(&state, &access_token, &base_url, &project_path).await?;
    let gitlab_issues = fetch_gitlab_issues(
        &state,
        &access_token,
        &base_url,
        gitlab_project.id,
        payload.include_closed_issues,
    )
    .await?;

    let project_created_at = gitlab_project
        .created_at
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let mut created_project_ids = MigrationRepository::bulk_create_projects(
        state.pool(),
        vec![MigrateProjectRequest {
            organization_id: payload.organization_id,
            name: gitlab_project.name.clone(),
            color: "217 91% 60%".to_string(),
            created_at: project_created_at,
        }],
    )
    .await
    .map_err(|error| {
        tracing::error!(?error, "failed to create project for GitLab import");
        ErrorResponse::new(StatusCode::INTERNAL_SERVER_ERROR, "failed to create project")
    })?;

    let project_id = created_project_ids.pop().ok_or_else(|| {
        ErrorResponse::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to resolve imported project id",
        )
    })?;

    ensure_project_access(state.pool(), ctx.user.id, project_id).await?;

    let migrate_issues = gitlab_issues
        .into_iter()
        .map(|issue| {
            let mut desc = issue.description.clone();
            if let Some(url) = issue.web_url {
                let link = format!("\n\nImported from GitLab issue: {url}");
                desc = Some(match desc {
                    Some(existing) if !existing.is_empty() => format!("{existing}{link}"),
                    _ => link.trim().to_string(),
                });
            }
            MigrateIssueRequest {
                project_id,
                status_name: if issue.state.eq_ignore_ascii_case("closed") {
                    "Done".to_string()
                } else {
                    "To do".to_string()
                },
                title: format!("#{} {}", issue.iid, issue.title),
                description: desc,
                created_at: DateTime::parse_from_rfc3339(&issue.created_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            }
        })
        .collect::<Vec<_>>();

    let imported_issues = if migrate_issues.is_empty() {
        0
    } else {
        MigrationRepository::bulk_create_issues(state.pool(), migrate_issues)
            .await
            .map_err(|error| {
                tracing::error!(?error, "failed to import GitLab issues");
                ErrorResponse::new(StatusCode::INTERNAL_SERVER_ERROR, "failed to import issues")
            })?
            .len()
    };

    Ok(Json(ImportGitLabProjectIssuesResponse {
        project_id,
        project_name: gitlab_project.name,
        imported_issues,
    }))
}
