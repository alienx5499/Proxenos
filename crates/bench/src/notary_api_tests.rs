use super::*;
use reqwest::header::{AUTHORIZATION, COOKIE};

fn headers(api: &Http) -> reqwest::header::HeaderMap {
    let req = api.authed(api.http.get(api.url("/x"))).build().unwrap();
    req.headers().clone()
}

#[test]
fn gateway_calls_send_the_login_cookie_not_a_bearer() {
    let h = headers(&Http::gateway("http://127.0.0.1:1", "jwt").unwrap());
    assert_eq!(h[COOKIE], "switch_auth=jwt");
    assert!(h.get(AUTHORIZATION).is_none());
}

#[test]
fn notary_calls_send_the_operator_bearer() {
    let h = headers(&Http::new("http://127.0.0.1:1", "op").unwrap());
    assert_eq!(h[AUTHORIZATION], "Bearer op");
    assert!(h.get(COOKIE).is_none());
}
