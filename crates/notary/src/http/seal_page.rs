//! The seal page, its two static assets, and the headers that keep the token private.

use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};

const PAGE: &str = include_str!("static/seal.html");
const SCRIPT: &str = include_str!("static/seal.js");
const STYLE: &str = include_str!("static/seal.css");

/// Only same-origin script, style and fetches; no framing, no base or foreign form targets.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; \
                   frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

fn reply(content_type: &'static str, body: &'static str) -> Response {
    let headers = [
        (header::CONTENT_TYPE, content_type),
        (header::CONTENT_SECURITY_POLICY, CSP),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::CACHE_CONTROL, "no-store"),
    ];
    (headers, body).into_response()
}

/// The token is in the URL, so the page sends no referrer and is not cached.
pub async fn page() -> Response {
    reply("text/html; charset=utf-8", PAGE)
}

pub async fn script() -> Response {
    reply("text/javascript; charset=utf-8", SCRIPT)
}

pub async fn style() -> Response {
    reply("text/css; charset=utf-8", STYLE)
}

/// For `/api/seal/{token}`: the schema and the join code must never be cached.
pub async fn no_store(mut res: Response) -> Response {
    let h = res.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    res
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::super::tests::app;

    async fn get(path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
        let req = Request::get(path).body(Body::empty()).unwrap();
        let resp = app().oneshot(req).await.unwrap();
        let (status, headers) = (resp.status(), resp.headers().clone());
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn page_and_assets_carry_csp_and_load_nothing_inline() {
        for (path, kind) in [
            ("/seal/abc", "text/html"),
            ("/static/seal.js", "text/javascript"),
            ("/static/seal.css", "text/css"),
        ] {
            let (status, h, _) = get(path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert!(h["content-type"].to_str().unwrap().starts_with(kind));
            let csp = h["content-security-policy"].to_str().unwrap();
            assert!(csp.contains("default-src 'none'") && csp.contains("frame-ancestors 'none'"));
            assert_eq!(h["x-content-type-options"], "nosniff");
            assert_eq!(h["referrer-policy"], "no-referrer");
        }
        let (_, _, html) = get("/seal/abc").await;
        assert!(html.contains("src=\"/static/seal.js\""));
        assert!(!html.contains("<style>") && !html.contains("<script>"));
        let (_, _, js) = get("/static/seal.js").await;
        assert!(js.contains("/api/seal/") && js.contains("if (sending) return;"));
    }

    #[tokio::test]
    async fn seal_api_responses_are_never_cached() {
        let (status, h, _) = get("/api/seal/unknown").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(h["cache-control"], "no-store");
    }
}
