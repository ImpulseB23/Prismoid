//! One-shot HTTP listener for the OAuth redirect.
//!
//! RFC 8252 §7.3 specifies loopback IP redirect for native apps:
//! the app binds an ephemeral port on `127.0.0.1`, registers
//! `http://127.0.0.1:<port>` as the redirect URI in the authorization
//! request, the OS browser delivers the redirect to that listener,
//! and the listener responds with a small HTML page telling the user
//! to switch back to the app.
//!
//! We deliberately bind `127.0.0.1` rather than `localhost` per the
//! same RFC: `localhost` resolves through the host's name resolver,
//! which on Windows can be configured to map to IPv6 `::1`, breaking
//! redirect_uri string-equality checks at the OAuth provider.
//!
//! Single-shot semantics: the listener accepts connections in a loop
//! but only resolves on the *first connection that carries a valid
//! OAuth redirect query*. Random probes (curl, port scans, browser
//! pre-fetch of the URL) get a `400 Bad Request` and the loop keeps
//! waiting. This avoids racing the user — the user's actual browser
//! redirect is what we want, not the first byte that hits the port.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::errors::PkceError;

/// Maximum bytes we'll read from a single inbound HTTP request before
/// declaring it malformed. The redirect request is `GET /?<query> HTTP/1.1`
/// plus headers — Chrome/Edge/Firefox all stay under 4 KiB. 8 KiB is
/// generous; anything larger is a signal someone is fuzzing the port.
const MAX_REQUEST_BYTES: usize = 8 * 1024;

/// Parsed OAuth redirect parameters. Either `code` + `state` (success)
/// or `error` (failure), per RFC 6749 §4.1.2 / §4.1.2.1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

impl RedirectParams {
    /// Returns `Err(PkceError::Authorization)` if the provider sent an
    /// `error=` param, otherwise the `code` and `state` fields. Used by
    /// the manager to short-circuit the state check on a denied flow.
    pub fn into_code_and_state(self) -> Result<(String, String), PkceError> {
        if let Some(err) = self.error {
            return Err(PkceError::Authorization(err));
        }
        let code = self
            .code
            .ok_or(PkceError::BadRequest("missing code parameter"))?;
        let state = self
            .state
            .ok_or(PkceError::BadRequest("missing state parameter"))?;
        Ok((code, state))
    }
}

/// Loopback HTTP listener pre-bound to `127.0.0.1:0` (OS picks port).
/// Hold one of these for the duration of an in-flight authorization
/// flow; calling [`LoopbackServer::wait_for_redirect`] consumes it.
pub struct LoopbackServer {
    listener: TcpListener,
    port: u16,
}

impl LoopbackServer {
    /// Bind a fresh loopback listener. The caller reads `port` /
    /// `redirect_uri` to construct the authorization URL before kicking
    /// off the browser launch.
    pub async fn bind() -> Result<Self, PkceError> {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let listener = TcpListener::bind(addr).await.map_err(PkceError::Bind)?;
        let port = listener.local_addr().map_err(PkceError::Bind)?.port();
        Ok(Self { listener, port })
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Full `http://127.0.0.1:<port>` redirect URI. This is what the
    /// caller passes as the `redirect_uri` query param on the
    /// authorization request and again on the token exchange — the two
    /// must match exactly per OAuth spec.
    #[must_use]
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Block until a valid OAuth redirect lands on the listener. Probes
    /// and malformed requests are answered with `400 Bad Request` and
    /// then ignored — only the first request that parses as a real
    /// OAuth redirect resolves the future.
    ///
    /// The caller is responsible for bounding wall time via
    /// `tokio::time::timeout`. We don't do it here because the caller
    /// often wants to cancel for unrelated reasons (UI close, logout)
    /// and the timeout can vary by provider's authorization page UX.
    pub async fn wait_for_redirect(self) -> Result<RedirectParams, PkceError> {
        loop {
            let (mut stream, _peer) = self.listener.accept().await.map_err(PkceError::Io)?;

            let mut buf = Vec::with_capacity(1024);
            // Read enough to parse the request line. We don't need the
            // body and HTTP/1.1 GETs don't have one. Cap at MAX_REQUEST_BYTES
            // so a slow loris can't pin us forever.
            let mut chunk = [0u8; 1024];
            loop {
                let n = stream.read(&mut chunk).await.map_err(PkceError::Io)?;
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                // Once we see the end of the request line + a blank
                // line (header terminator) we have everything we need.
                if find_double_crlf(&buf).is_some() {
                    break;
                }
                if buf.len() >= MAX_REQUEST_BYTES {
                    break;
                }
            }

            match parse_redirect(&buf) {
                Ok(params) => {
                    write_success_page(&mut stream).await.ok();
                    let _ = stream.shutdown().await;
                    return Ok(params);
                }
                Err(_) => {
                    write_400(&mut stream).await.ok();
                    let _ = stream.shutdown().await;
                    // Keep waiting for the real request.
                }
            }
        }
    }
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Parse the request line out of the inbound bytes and pull `code`,
/// `state`, `error` out of the query string. Returns
/// `PkceError::BadRequest` for anything that doesn't look like a
/// browser-issued GET to our redirect path.
fn parse_redirect(buf: &[u8]) -> Result<RedirectParams, PkceError> {
    let request_line_end = buf
        .windows(2)
        .position(|w| w == b"\r\n")
        .ok_or(PkceError::BadRequest("no CRLF in request"))?;
    let line = std::str::from_utf8(&buf[..request_line_end])
        .map_err(|_| PkceError::BadRequest("non-utf8 request line"))?;

    // Format: METHOD SP REQUEST-TARGET SP HTTP-VERSION
    let mut parts = line.split(' ');
    let method = parts
        .next()
        .ok_or(PkceError::BadRequest("missing method"))?;
    let target = parts
        .next()
        .ok_or(PkceError::BadRequest("missing request target"))?;

    if method != "GET" {
        return Err(PkceError::BadRequest("non-GET request"));
    }

    // Target looks like `/?code=...&state=...` or `/favicon.ico`.
    // Anything without a query is treated as a probe.
    let query = match target.split_once('?') {
        Some((_, q)) => q,
        None => return Err(PkceError::BadRequest("no query string")),
    };

    let mut params = RedirectParams {
        code: None,
        state: None,
        error: None,
    };

    for kv in query.split('&') {
        let (k, v) = match kv.split_once('=') {
            Some((k, v)) => (k, v),
            None => continue,
        };
        let decoded = percent_decode(v);
        match k {
            "code" => params.code = Some(decoded),
            "state" => params.state = Some(decoded),
            "error" => params.error = Some(decoded),
            _ => {}
        }
    }

    if params.code.is_none() && params.error.is_none() {
        return Err(PkceError::BadRequest(
            "neither code nor error in query string",
        ));
    }

    Ok(params)
}

/// Decode `%XX` sequences and `+`-as-space in a query-string value.
/// Hand-rolled rather than pulling a dependency for ~30 lines.
/// Invalid sequences pass through untouched — the OAuth provider's
/// `code` is opaque base64-ish text and unlikely to contain them, but
/// we'd rather see the raw value than 500 the redirect on a malformed
/// percent-escape.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_digit(bytes[i + 1]);
                let lo = hex_digit(bytes[i + 2]);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h << 4) | l);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// HTML body shown to the user after a successful redirect. Kept small
/// and self-contained — no external assets, no framework, no JS. The
/// tone matches the Twitch DCF flow's "switch back to the app" prompt.
const SUCCESS_PAGE: &str = include_str!("success_page.html");

async fn write_success_page<W: AsyncWriteExt + Unpin>(w: &mut W) -> std::io::Result<()> {
    let body = SUCCESS_PAGE.as_bytes();
    let header = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Cache-Control: no-store\r\n\
         \r\n",
        body.len()
    );
    w.write_all(header.as_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await
}

async fn write_400<W: AsyncWriteExt + Unpin>(w: &mut W) -> std::io::Result<()> {
    let body = b"bad request\n";
    let header = format!(
        "HTTP/1.1 400 Bad Request\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    w.write_all(header.as_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;

    #[test]
    fn parse_redirect_extracts_code_and_state() {
        let raw = b"GET /?code=abc123&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let p = parse_redirect(raw).unwrap();
        assert_eq!(p.code.as_deref(), Some("abc123"));
        assert_eq!(p.state.as_deref(), Some("xyz"));
        assert!(p.error.is_none());
    }

    #[test]
    fn parse_redirect_extracts_error() {
        let raw = b"GET /?error=access_denied HTTP/1.1\r\n\r\n";
        let p = parse_redirect(raw).unwrap();
        assert_eq!(p.error.as_deref(), Some("access_denied"));
        assert!(p.code.is_none());
    }

    #[test]
    fn parse_redirect_handles_percent_encoding() {
        let raw = b"GET /?code=a%2Fb%3Dc&state=hello%20world HTTP/1.1\r\n\r\n";
        let p = parse_redirect(raw).unwrap();
        assert_eq!(p.code.as_deref(), Some("a/b=c"));
        assert_eq!(p.state.as_deref(), Some("hello world"));
    }

    #[test]
    fn parse_redirect_handles_plus_as_space_in_value() {
        let raw = b"GET /?code=hello+world&state=s HTTP/1.1\r\n\r\n";
        let p = parse_redirect(raw).unwrap();
        assert_eq!(p.code.as_deref(), Some("hello world"));
    }

    #[test]
    fn parse_redirect_rejects_non_get() {
        let raw = b"POST /?code=abc HTTP/1.1\r\n\r\n";
        assert!(matches!(
            parse_redirect(raw),
            Err(PkceError::BadRequest("non-GET request"))
        ));
    }

    #[test]
    fn parse_redirect_rejects_no_query() {
        let raw = b"GET /favicon.ico HTTP/1.1\r\n\r\n";
        assert!(matches!(
            parse_redirect(raw),
            Err(PkceError::BadRequest("no query string"))
        ));
    }

    #[test]
    fn parse_redirect_rejects_query_without_code_or_error() {
        let raw = b"GET /?nonsense=1 HTTP/1.1\r\n\r\n";
        assert!(matches!(parse_redirect(raw), Err(PkceError::BadRequest(_))));
    }

    #[test]
    fn into_code_and_state_propagates_authorization_error() {
        let p = RedirectParams {
            code: None,
            state: None,
            error: Some("access_denied".into()),
        };
        assert!(matches!(
            p.into_code_and_state(),
            Err(PkceError::Authorization(s)) if s == "access_denied"
        ));
    }

    #[test]
    fn into_code_and_state_requires_state_param() {
        let p = RedirectParams {
            code: Some("c".into()),
            state: None,
            error: None,
        };
        assert!(matches!(
            p.into_code_and_state(),
            Err(PkceError::BadRequest("missing state parameter"))
        ));
    }

    #[test]
    fn percent_decode_passes_through_invalid_escape() {
        // Invalid % sequence — pass `%` through and continue.
        assert_eq!(percent_decode("a%ZZb"), "a%ZZb");
    }

    #[test]
    fn find_double_crlf_locates_terminator() {
        let buf = b"GET / HTTP/1.1\r\nHost: x\r\n\r\nbody";
        let pos = find_double_crlf(buf).unwrap();
        assert_eq!(&buf[pos..pos + 4], b"\r\n\r\n");
    }

    #[tokio::test]
    async fn loopback_returns_first_valid_redirect_after_probe() {
        let server = LoopbackServer::bind().await.unwrap();
        let port = server.port();
        let server_task = tokio::spawn(server.wait_for_redirect());

        // Send a probe that should be ignored.
        let mut probe = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        probe
            .write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut sink = Vec::new();
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut probe, &mut sink).await;

        // Now the real redirect.
        let mut real = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        real.write_all(b"GET /?code=abc&state=xyz HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut sink = Vec::new();
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut real, &mut sink).await;

        let params = server_task.await.unwrap().unwrap();
        assert_eq!(params.code.as_deref(), Some("abc"));
        assert_eq!(params.state.as_deref(), Some("xyz"));
    }

    #[tokio::test]
    async fn loopback_returns_error_param_without_code() {
        let server = LoopbackServer::bind().await.unwrap();
        let port = server.port();
        let server_task = tokio::spawn(server.wait_for_redirect());

        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(b"GET /?error=access_denied HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut sink = Vec::new();
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut s, &mut sink).await;

        let params = server_task.await.unwrap().unwrap();
        assert_eq!(params.error.as_deref(), Some("access_denied"));
    }

    #[tokio::test]
    async fn redirect_uri_uses_127_0_0_1() {
        let server = LoopbackServer::bind().await.unwrap();
        let uri = server.redirect_uri();
        assert!(uri.starts_with("http://127.0.0.1:"));
        assert!(uri.contains(&server.port().to_string()));
    }
}
