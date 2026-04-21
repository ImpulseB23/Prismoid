//! Generic POST-form helpers for OAuth code-for-token and refresh-token
//! exchanges.
//!
//! Per RFC 6749 §4.1.3 the token request is `application/x-www-form-urlencoded`
//! and the response is JSON. Provider-specific differences (extra
//! params, header requirements, error code vocabularies) are bridged
//! by the caller passing in the right `params` slice and classifying
//! the returned `error` string from `PkceError::TokenEndpoint`.

use serde::Deserialize;

use super::errors::PkceError;

/// Subset of RFC 6749 §5.1 that we use. Providers add fields (Google
/// adds `id_token` for OpenID flows) that we ignore via serde's
/// permissive default.
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    /// Optional because refresh exchanges sometimes don't rotate the
    /// refresh token — caller falls back to the previously stored value.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Seconds until access_token expires. Caller converts to absolute
    /// `expires_at_ms` so a sleeping process re-evaluates correctly.
    pub expires_in: u64,
    /// Space-delimited per RFC 6749 §3.3. Some providers omit this on
    /// refresh; caller falls back to previously stored scopes.
    #[serde(default)]
    pub scope: Option<String>,
    /// Always `Bearer` in practice but present per spec.
    #[serde(default)]
    pub token_type: Option<String>,
}

/// Provider-specific error envelope (RFC 6749 §5.2). Both fields are
/// optional in practice — Google always sets both, but we don't want
/// to require them so a stripped-down provider response still surfaces
/// a meaningful message.
#[derive(Debug, Deserialize)]
struct ErrorResponse {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// Exchange an authorization code for tokens. Caller supplies the full
/// `params` slice (provider-specific keys: `client_id`, `client_secret`
/// when required, `code`, `code_verifier`, `redirect_uri`, etc.) so this
/// helper stays provider-agnostic.
pub async fn exchange_code(
    http: &reqwest::Client,
    token_endpoint: &str,
    params: &[(&str, &str)],
) -> Result<TokenResponse, PkceError> {
    post_form(http, token_endpoint, params).await
}

/// Refresh-token exchange. Same shape as [`exchange_code`] but with
/// `grant_type=refresh_token` plus the stored `refresh_token` and
/// provider credentials.
pub async fn refresh_tokens(
    http: &reqwest::Client,
    token_endpoint: &str,
    params: &[(&str, &str)],
) -> Result<TokenResponse, PkceError> {
    post_form(http, token_endpoint, params).await
}

async fn post_form(
    http: &reqwest::Client,
    endpoint: &str,
    params: &[(&str, &str)],
) -> Result<TokenResponse, PkceError> {
    let body = encode_form(params);
    let resp = http
        .post(endpoint)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(reqwest::header::ACCEPT, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| PkceError::Http(e.to_string()))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| PkceError::Http(e.to_string()))?;

    if status.is_success() {
        return serde_json::from_str(&body).map_err(|e| PkceError::Decode(e.to_string()));
    }

    // Try to surface the provider's `error_description` first; fall
    // back to `error`; fall back to the raw body so the caller's logs
    // are useful even on a totally unknown shape.
    let parsed: Option<ErrorResponse> = serde_json::from_str(&body).ok();
    let msg = parsed
        .as_ref()
        .and_then(|e| e.error_description.clone().or_else(|| e.error.clone()))
        .unwrap_or_else(|| body.trim().to_string());
    Err(PkceError::TokenEndpoint(msg))
}

/// Percent-encode an OAuth token-endpoint form body. We can't rely on
/// reqwest's `.form()` helper because it lives behind a feature flag
/// that's off in our minimal feature set, and pulling `serde_urlencoded`
/// in for two call sites would be wasteful — RFC 3986 unreserved set
/// is small and the encoding is mechanical.
fn encode_form(params: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (i, (k, v)) in params.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        percent_encode_into(k, &mut out);
        out.push('=');
        percent_encode_into(v, &mut out);
    }
    out
}

fn percent_encode_into(s: &str, out: &mut String) {
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            other => {
                out.push('%');
                out.push(hex_nibble(other >> 4));
                out.push(hex_nibble(other & 0x0f));
            }
        }
    }
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'A' + n - 10) as char,
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client")
    }

    #[tokio::test]
    async fn exchange_code_decodes_minimal_response() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST).path("/token");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(r#"{"access_token":"at","expires_in":3600}"#);
            })
            .await;
        let url = format!("{}/token", server.base_url());
        let resp = exchange_code(&http_client(), &url, &[("foo", "bar")])
            .await
            .unwrap();
        assert_eq!(resp.access_token, "at");
        assert_eq!(resp.expires_in, 3600);
        assert!(resp.refresh_token.is_none());
    }

    #[tokio::test]
    async fn exchange_code_returns_provider_error_description() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST).path("/token");
                then.status(400)
                    .header("content-type", "application/json")
                    .body(r#"{"error":"invalid_grant","error_description":"bad code"}"#);
            })
            .await;
        let url = format!("{}/token", server.base_url());
        let err = exchange_code(&http_client(), &url, &[("foo", "bar")])
            .await
            .unwrap_err();
        match err {
            PkceError::TokenEndpoint(msg) => assert_eq!(msg, "bad code"),
            other => panic!("expected TokenEndpoint, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn exchange_code_falls_back_to_error_field_without_description() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST).path("/token");
                then.status(400)
                    .header("content-type", "application/json")
                    .body(r#"{"error":"invalid_request"}"#);
            })
            .await;
        let url = format!("{}/token", server.base_url());
        let err = exchange_code(&http_client(), &url, &[]).await.unwrap_err();
        match err {
            PkceError::TokenEndpoint(msg) => assert_eq!(msg, "invalid_request"),
            other => panic!("expected TokenEndpoint, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn exchange_code_falls_back_to_raw_body_on_unparsable_error() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST).path("/token");
                then.status(500).body("internal server error");
            })
            .await;
        let url = format!("{}/token", server.base_url());
        let err = exchange_code(&http_client(), &url, &[]).await.unwrap_err();
        match err {
            PkceError::TokenEndpoint(msg) => assert_eq!(msg, "internal server error"),
            other => panic!("expected TokenEndpoint, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn refresh_tokens_decodes_with_rotated_refresh() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST).path("/token");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        r#"{"access_token":"at2","refresh_token":"rt2","expires_in":1800,"scope":"a b","token_type":"Bearer"}"#,
                    );
            })
            .await;
        let url = format!("{}/token", server.base_url());
        let resp = refresh_tokens(&http_client(), &url, &[]).await.unwrap();
        assert_eq!(resp.refresh_token.as_deref(), Some("rt2"));
        assert_eq!(resp.scope.as_deref(), Some("a b"));
    }

    #[tokio::test]
    async fn exchange_code_decode_error_for_non_json_success() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST).path("/token");
                then.status(200).body("not json");
            })
            .await;
        let url = format!("{}/token", server.base_url());
        let err = exchange_code(&http_client(), &url, &[]).await.unwrap_err();
        assert!(matches!(err, PkceError::Decode(_)));
    }

    #[test]
    fn encode_form_round_trips_single_pair() {
        let body = encode_form(&[("grant_type", "refresh_token")]);
        assert_eq!(body, "grant_type=refresh_token");
    }

    #[test]
    fn encode_form_joins_pairs_with_ampersand() {
        let body = encode_form(&[("a", "1"), ("b", "2"), ("c", "3")]);
        assert_eq!(body, "a=1&b=2&c=3");
    }

    #[test]
    fn encode_form_percent_encodes_reserved_chars() {
        // Space, slash, plus, equals must all encode.
        let body = encode_form(&[("scope", "a b/c+d=e")]);
        assert_eq!(body, "scope=a%20b%2Fc%2Bd%3De");
    }

    #[test]
    fn encode_form_passes_through_unreserved_set() {
        let body = encode_form(&[("k", "AZaz09-._~")]);
        assert_eq!(body, "k=AZaz09-._~");
    }

    #[test]
    fn encode_form_handles_empty_value() {
        let body = encode_form(&[("k", "")]);
        assert_eq!(body, "k=");
    }
}
