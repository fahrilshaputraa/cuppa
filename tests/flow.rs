// SPDX-License-Identifier: AGPL-3.0-or-later
//! End-to-end tests against the real router and a real (temp-file) SQLite database.

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use cuppa::{AppState, config::Config, router};
use http_body_util::BodyExt;
use std::net::SocketAddr;
use tower::ServiceExt;

struct App {
    app: Router,
    st: AppState,
    code: String,
}

async fn app() -> App {
    let dir = std::env::temp_dir().join(format!("cuppa-test-{}", cuppa::rand_id::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = Config::for_tests();
    cfg.database_url = format!("sqlite://{}/t.db", dir.display());
    let st = AppState::new(cfg).await.unwrap();
    let code = st.setup_code.lock().unwrap().clone().unwrap();
    App {
        app: router(st.clone()),
        st,
        code,
    }
}

struct Res {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: String,
}

async fn send(a: &App, method: &str, uri: &str, cookie: Option<&str>, form: Option<&str>, origin: bool) -> Res {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost:8080");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    if origin && method == "POST" {
        b = b.header(header::ORIGIN, "http://localhost:8080");
    }
    if form.is_some() {
        b = b.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    }
    let mut req = b.body(Body::from(form.unwrap_or("").to_string())).unwrap();
    req.extensions_mut()
        .insert(ConnectInfo("10.0.0.9:5555".parse::<SocketAddr>().unwrap()));
    let r = a.app.clone().oneshot(req).await.unwrap();
    let (p, body) = r.into_parts();
    Res {
        status: p.status,
        headers: p.headers,
        body: String::from_utf8_lossy(&body.collect().await.unwrap().to_bytes()).into(),
    }
}

fn cookie_of(r: &Res) -> String {
    r.headers
        .get_all(header::SET_COOKIE)
        .iter()
        .next()
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

fn loc(r: &Res) -> String {
    r.headers.get(header::LOCATION).unwrap().to_str().unwrap().to_string()
}

async fn setup_owner(a: &App) -> String {
    let f = format!("code={}&outlet=Kopi+Test&name=Ana&secret=correct+horse+battery", a.code);
    let r = send(a, "POST", "/setup", None, Some(&f), true).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    cookie_of(&r)
}

async fn login(a: &App, name: &str, secret: &str) -> Res {
    send(
        a,
        "POST",
        "/login",
        None,
        Some(&format!("name={name}&secret={secret}")),
        true,
    )
    .await
}

/// Owner creates menu + table + cashier; returns (table token, menu item id, cashier cookie).
async fn seed(a: &App, owner: &str) -> (String, String, String) {
    send(a, "POST", "/owner/categories", Some(owner), Some("name=Kopi"), true).await;
    let cat: String = sqlx::query_scalar("SELECT id FROM categories")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    send(
        a,
        "POST",
        "/owner/items",
        Some(owner),
        Some(&format!("category_id={cat}&name=Latte&price=25000")),
        true,
    )
    .await;
    send(a, "POST", "/owner/tables", Some(owner), Some("label=1"), true).await;
    send(
        a,
        "POST",
        "/owner/staff",
        Some(owner),
        Some("name=Budi&role=cashier&secret=1234"),
        true,
    )
    .await;
    let item: String = sqlx::query_scalar("SELECT id FROM menu_items")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    let token: String = sqlx::query_scalar("SELECT token FROM tables")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    let c = login(a, "Budi", "1234").await;
    (token, item, cookie_of(&c))
}

#[tokio::test]
async fn unauthenticated_staff_pages_redirect_to_login() {
    let a = app().await;
    for p in ["/owner", "/cashier", "/cashier/queue", "/owner/audit", "/events"] {
        let r = send(&a, "GET", p, None, None, false).await;
        assert_eq!(r.status, StatusCode::SEE_OTHER, "{p}");
        assert_eq!(loc(&r), "/login");
    }
}

#[tokio::test]
async fn setup_needs_code_and_works_only_once() {
    let a = app().await;
    let bad = send(
        &a,
        "POST",
        "/setup",
        None,
        Some("code=nope&outlet=X&name=A&secret=correct+horse+battery"),
        true,
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    setup_owner(&a).await;
    // second attempt: setup is closed for good
    let again = send(
        &a,
        "POST",
        "/setup",
        None,
        Some(&format!(
            "code={}&outlet=X&name=Eve&secret=correct+horse+battery",
            a.code
        )),
        true,
    )
    .await;
    assert_eq!(again.status, StatusCode::SEE_OTHER);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn security_headers_and_cookie_flags() {
    let a = app().await;
    setup_owner(&a).await;
    let r = login(&a, "Ana", "correct+horse+battery").await;
    let sc = r.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(sc.contains("HttpOnly") && sc.contains("SameSite=Strict"), "{sc}");
    let page = send(&a, "GET", "/login", None, None, false).await;
    for h in [
        "content-security-policy",
        "x-content-type-options",
        "x-frame-options",
        "referrer-policy",
        "cache-control",
    ] {
        assert!(page.headers.contains_key(h), "missing {h}");
    }
    assert!(
        page.headers["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("script-src 'self'")
    );
    assert!(!page.body.contains("<script>"), "no inline script allowed");
}

#[tokio::test]
async fn csrf_blocks_cross_origin_posts() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let r = send(&a, "POST", "/owner/categories", Some(&owner), Some("name=Evil"), false).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let mut req = Request::builder()
        .method("POST")
        .uri("/owner/categories")
        .header(header::HOST, "localhost:8080")
        .header(header::ORIGIN, "https://evil.example")
        .header(header::COOKIE, &owner)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("name=Evil"))
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo("10.0.0.9:1".parse::<SocketAddr>().unwrap()));
    assert_eq!(
        a.app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn pin_lockout_after_five_failures() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    send(
        &a,
        "POST",
        "/owner/staff",
        Some(&owner),
        Some("name=Budi&role=cashier&secret=1234"),
        true,
    )
    .await;
    for _ in 0..5 {
        assert_eq!(login(&a, "Budi", "9999").await.status, StatusCode::UNAUTHORIZED);
    }
    // even the right PIN is refused while locked
    let r = login(&a, "Budi", "1234").await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(r.body.contains("dikunci"));
}

#[tokio::test]
async fn role_enforced_on_server_not_only_hidden_in_ui() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (_, _, cashier) = seed(&a, &owner).await;
    for p in ["/owner", "/owner/menu", "/owner/audit", "/owner/staff"] {
        assert_eq!(
            send(&a, "GET", p, Some(&cashier), None, false).await.status,
            StatusCode::FORBIDDEN,
            "{p}"
        );
    }
    let r = send(
        &a,
        "POST",
        "/owner/items",
        Some(&cashier),
        Some("category_id=x&name=Hack&price=1"),
        true,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn customer_order_is_repriced_confirmed_paid_and_locked() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (token, item, cashier) = seed(&a, &owner).await;

    // Customer tries to smuggle a price: only quantities are read, total is computed by the server (10% tax).
    let r = send(
        &a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=2&price_{item}=1&total=1")),
        true,
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    let (oid, status, total): (String, String, i64) = sqlx::query_as("SELECT id, status, total FROM orders")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    assert_eq!((status.as_str(), total), ("pending", 55_000)); // 50.000 + 10% tax

    // Cannot pay before the cashier confirms.
    let early = send(
        &a,
        "POST",
        &format!("/orders/{oid}/pay"),
        Some(&cashier),
        Some("method=cash&tendered=60000"),
        true,
    )
    .await;
    assert_eq!(early.status, StatusCode::UNPROCESSABLE_ENTITY);

    send(
        &a,
        "POST",
        &format!("/orders/{oid}/confirm"),
        Some(&cashier),
        Some(""),
        true,
    )
    .await;
    // No open shift -> payment refused.
    let noshift = send(
        &a,
        "POST",
        &format!("/orders/{oid}/pay"),
        Some(&cashier),
        Some("method=cash&tendered=60000"),
        true,
    )
    .await;
    assert_eq!(noshift.status, StatusCode::UNPROCESSABLE_ENTITY);

    send(
        &a,
        "POST",
        "/cashier/shift/open",
        Some(&cashier),
        Some("opening=100000"),
        true,
    )
    .await;
    let short = send(
        &a,
        "POST",
        &format!("/orders/{oid}/pay"),
        Some(&cashier),
        Some("method=cash&tendered=10000"),
        true,
    )
    .await;
    assert!(short.body.contains("kurang"));
    let paid = send(
        &a,
        "POST",
        &format!("/orders/{oid}/pay"),
        Some(&cashier),
        Some("method=cash&tendered=60000"),
        true,
    )
    .await;
    assert_eq!(paid.status, StatusCode::SEE_OTHER);

    // Price change does not rewrite history.
    send(
        &a,
        "POST",
        "/owner/items",
        Some(&owner),
        Some("category_id=x&name=y&price=1"),
        true,
    )
    .await;
    sqlx::query("UPDATE menu_items SET price = 99999")
        .execute(&a.st.db)
        .await
        .unwrap();
    let t: i64 = sqlx::query_scalar("SELECT total FROM orders WHERE id = ?")
        .bind(&oid)
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    assert_eq!(t, 55_000);

    // The database itself refuses to alter a paid order or its items, and refuses payments/audit tampering.
    assert!(
        sqlx::query("UPDATE orders SET total = 1 WHERE id = ?")
            .bind(&oid)
            .execute(&a.st.db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE order_items SET qty = 9 WHERE order_id = ?")
            .bind(&oid)
            .execute(&a.st.db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("INSERT INTO order_items (id,order_id,menu_id,name,unit_price,qty) VALUES ('z',?,?,'x',1,1)")
            .bind(&oid)
            .bind(&item)
            .execute(&a.st.db)
            .await
            .is_err()
    );
    assert!(sqlx::query("DELETE FROM payments").execute(&a.st.db).await.is_err());
    assert!(
        sqlx::query("UPDATE audit_log SET action = 'x'")
            .execute(&a.st.db)
            .await
            .is_err()
    );
    assert!(sqlx::query("DELETE FROM audit_log").execute(&a.st.db).await.is_err());
    assert!(sqlx::query("DELETE FROM orders").execute(&a.st.db).await.is_err());
}

#[tokio::test]
async fn void_needs_a_different_owner_and_is_audited() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (token, item, cashier) = seed(&a, &owner).await;
    send(
        &a,
        "POST",
        "/owner/staff",
        Some(&owner),
        Some("name=Citra&role=owner&secret=another+long+pass"),
        true,
    )
    .await;
    send(
        &a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=1")),
        true,
    )
    .await;
    let oid: String = sqlx::query_scalar("SELECT id FROM orders")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    send(
        &a,
        "POST",
        &format!("/orders/{oid}/confirm"),
        Some(&cashier),
        Some(""),
        true,
    )
    .await;
    send(
        &a,
        "POST",
        "/cashier/shift/open",
        Some(&cashier),
        Some("opening=0"),
        true,
    )
    .await;
    send(
        &a,
        "POST",
        &format!("/orders/{oid}/pay"),
        Some(&cashier),
        Some("method=qris&reference=ABC123"),
        true,
    )
    .await;

    // wrong secret
    let bad = send(
        &a,
        "POST",
        &format!("/orders/{oid}/void"),
        Some(&cashier),
        Some("reason=salah&approver=Citra&secret=nope"),
        true,
    )
    .await;
    assert!(bad.body.contains("salah atau terkunci"));
    // a cashier cannot approve their own void
    let selfv = send(
        &a,
        "POST",
        &format!("/orders/{oid}/void"),
        Some(&cashier),
        Some("reason=x&approver=Budi&secret=1234"),
        true,
    )
    .await;
    assert!(selfv.body.contains("owner lain"));
    let ok = send(
        &a,
        "POST",
        &format!("/orders/{oid}/void"),
        Some(&cashier),
        Some("reason=salah+input&approver=Citra&secret=another+long+pass"),
        true,
    )
    .await;
    assert_eq!(ok.status, StatusCode::SEE_OTHER);
    let st: String = sqlx::query_scalar("SELECT status FROM orders WHERE id = ?")
        .bind(&oid)
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    assert_eq!(st, "voided");
    let (approver, reason): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT approver_id, reason FROM audit_log WHERE action = 'order.voided'")
            .fetch_one(&a.st.db)
            .await
            .unwrap();
    assert!(approver.is_some() && reason.as_deref() == Some("salah input"));
}

#[tokio::test]
async fn customer_guards_unknown_token_soldout_and_spam() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (token, item, _) = seed(&a, &owner).await;
    assert_eq!(
        send(&a, "GET", &format!("/m/{}", "0".repeat(64)), None, None, false)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&a, "GET", "/m/short", None, None, false).await.status,
        StatusCode::NOT_FOUND
    );
    // sold out
    send(
        &a,
        "POST",
        &format!("/owner/items/{item}/available"),
        Some(&owner),
        Some(""),
        true,
    )
    .await;
    let r = send(
        &a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=1")),
        true,
    )
    .await;
    assert!(r.body.contains("habis"));
    // absurd quantity
    send(
        &a,
        "POST",
        &format!("/owner/items/{item}/available"),
        Some(&owner),
        Some(""),
        true,
    )
    .await;
    let r = send(
        &a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=500")),
        true,
    )
    .await;
    assert!(r.body.contains("batas"));
    // rotating the token kills the old QR
    let tid: String = sqlx::query_scalar("SELECT id FROM tables")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    send(
        &a,
        "POST",
        &format!("/owner/tables/{tid}/rotate"),
        Some(&owner),
        Some(""),
        true,
    )
    .await;
    assert_eq!(
        send(&a, "GET", &format!("/m/{token}"), None, None, false).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn blind_shift_close_reports_difference() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (token, item, cashier) = seed(&a, &owner).await;
    send(
        &a,
        "POST",
        "/cashier/shift/open",
        Some(&cashier),
        Some("opening=100000"),
        true,
    )
    .await;
    send(
        &a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=2")),
        true,
    )
    .await;
    let oid: String = sqlx::query_scalar("SELECT id FROM orders")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    send(
        &a,
        "POST",
        &format!("/orders/{oid}/confirm"),
        Some(&cashier),
        Some(""),
        true,
    )
    .await;
    send(
        &a,
        "POST",
        &format!("/orders/{oid}/pay"),
        Some(&cashier),
        Some("method=cash&tendered=55000"),
        true,
    )
    .await;
    // The shift page before closing never reveals the expected figure.
    let page = send(&a, "GET", "/cashier/shift", Some(&cashier), None, false).await;
    assert!(!page.body.contains("155.000"));
    let r = send(
        &a,
        "POST",
        "/cashier/shift/close",
        Some(&cashier),
        Some("counted=150000"),
        true,
    )
    .await;
    assert!(
        r.body.contains("Rp155.000") && r.body.contains("-Rp5.000"),
        "{}",
        r.body
    );
}
