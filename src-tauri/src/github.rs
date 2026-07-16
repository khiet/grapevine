use serde::Deserialize;
use serde_json::{json, Value};

const GRAPHQL_URL: &str = "https://api.github.com/graphql";

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: String,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphQlResponse {
    data: Option<Value>,
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

/// Errors are user-facing strings: they surface as inline messages in the
/// settings view, so they must stand on their own without context.
async fn graphql(token: &str, query: &str, variables: Value) -> Result<GraphQlResponse, String> {
    let client = reqwest::Client::builder()
        .user_agent("grapevine")
        .build()
        .map_err(|e| format!("cannot build HTTP client: {e}"))?;
    let response = client
        .post(GRAPHQL_URL)
        .bearer_auth(token)
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .map_err(|_| "Could not reach GitHub. Check your network connection.".to_string())?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("GitHub rejected the token. Check that it is valid and not expired.".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "GitHub returned an error (HTTP {}).",
            response.status().as_u16()
        ));
    }
    response
        .json::<GraphQlResponse>()
        .await
        .map_err(|_| "GitHub returned an unexpected response.".to_string())
}

/// Validates the token by asking GitHub who it belongs to. Returns the login.
pub async fn validate_token(token: &str) -> Result<String, String> {
    let response = graphql(token, "query { viewer { login } }", json!({})).await?;
    if let Some(login) = response
        .data
        .as_ref()
        .and_then(|d| d.pointer("/viewer/login"))
        .and_then(Value::as_str)
    {
        return Ok(login.to_string());
    }
    Err(response
        .errors
        .into_iter()
        .next()
        .map(|e| format!("GitHub error: {}", e.message))
        .unwrap_or_else(|| "GitHub returned an unexpected response.".into()))
}

/// Checks that `owner/name` exists and is accessible with this token.
/// Returns the canonical `nameWithOwner` (fixes user-typed casing).
pub async fn validate_repo(token: &str, owner: &str, name: &str) -> Result<String, String> {
    let response = graphql(
        token,
        "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { nameWithOwner } }",
        json!({ "owner": owner, "name": name }),
    )
    .await?;
    if let Some(full_name) = response
        .data
        .as_ref()
        .and_then(|d| d.pointer("/repository/nameWithOwner"))
        .and_then(Value::as_str)
    {
        return Ok(full_name.to_string());
    }
    if response
        .errors
        .iter()
        .any(|e| e.kind.as_deref() == Some("NOT_FOUND"))
    {
        return Err(format!(
            "{owner}/{name} was not found or is not accessible with this token."
        ));
    }
    Err(response
        .errors
        .into_iter()
        .next()
        .map(|e| format!("GitHub error: {}", e.message))
        .unwrap_or_else(|| "GitHub returned an unexpected response.".into()))
}
