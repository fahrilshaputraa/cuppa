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

async fn login_with(a: &App, name: &str, secret: &str, cookie: &str) -> Res {
    send(
        a,
        "POST",
        "/login",
        Some(cookie),
        Some(&format!("name={name}&secret={secret}")),
        true,
    )
    .await
}

/// Owner registers "this device"; returns the device cookie.
async fn register_device(a: &App, owner: &str) -> String {
    let r = send(
        a,
        "POST",
        "/owner/devices",
        Some(owner),
        Some("name=Tablet+kasir"),
        true,
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    cookie_of(&r)
}

async fn setup_owner(a: &App) -> String {
    let f = format!("code={}&outlet=Kopi+Test&name=Ana&secret=correct+horse+battery", a.code);
    let r = send(a, "POST", "/setup", None, Some(&f), true).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    let owner = cookie_of(&r);
    let w = send(
        a,
        "POST",
        "/owner/wizard",
        Some(&owner),
        Some("qr_ordering=1&kitchen_mode=0&tax_pct=10&require_device=1"),
        true,
    )
    .await;
    assert_eq!(w.status, StatusCode::SEE_OTHER, "{}", w.body);
    owner
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
    let dev = register_device(a, owner).await;
    let c = login_with(a, "Budi", "1234", &dev).await;
    assert_eq!(c.status, StatusCode::SEE_OTHER, "{}", c.body);
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
    assert!(selfv.body.contains("bukan kasir"));
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

async fn paid_order(a: &App, owner: &str) -> (String, String) {
    let (token, item, cashier) = seed(a, owner).await;
    send(
        a,
        "POST",
        "/cashier/shift/open",
        Some(&cashier),
        Some("opening=0"),
        true,
    )
    .await;
    send(
        a,
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
        a,
        "POST",
        &format!("/orders/{oid}/confirm"),
        Some(&cashier),
        Some(""),
        true,
    )
    .await;
    let r = send(
        a,
        "POST",
        &format!("/orders/{oid}/pay"),
        Some(&cashier),
        Some("method=cash&tendered=100000"),
        true,
    )
    .await;
    assert_eq!(loc(&r), format!("/orders/{oid}/receipt"));
    (oid, cashier)
}

#[tokio::test]
async fn receipt_reports_csv_and_backup() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (oid, cashier) = paid_order(&a, &owner).await;

    let rc = send(
        &a,
        "GET",
        &format!("/orders/{oid}/receipt"),
        Some(&cashier),
        None,
        false,
    )
    .await;
    assert!(
        rc.body.contains("Rp55.000") && rc.body.contains("Kembali") && rc.body.contains("Rp45.000"),
        "{}",
        rc.body
    );

    let rep = send(&a, "GET", "/owner/reports", Some(&owner), None, false).await;
    assert_eq!(rep.status, StatusCode::OK);
    assert!(rep.body.contains("Rp55.000") && rep.body.contains("Latte") && rep.body.contains("Tunai"));
    let csv = send(&a, "GET", "/owner/reports.csv", Some(&owner), None, false).await;
    assert!(csv.body.starts_with("day,number") && csv.body.contains("55000"));
    assert!(
        csv.headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
    assert_eq!(
        send(&a, "GET", "/owner/reports?from=bad", Some(&owner), None, false)
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Reports and backups are owner-only.
    for p in ["/owner/reports", "/owner/reports.csv"] {
        assert_eq!(
            send(&a, "GET", p, Some(&cashier), None, false).await.status,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        send(&a, "POST", "/owner/backup", Some(&cashier), Some(""), true)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let b = send(&a, "POST", "/owner/backup", Some(&owner), Some(""), true).await;
    assert_eq!(b.status, StatusCode::OK);
    assert!(b.body.starts_with("SQLite format 3"));
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE action='backup.downloaded'")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn printer_must_be_on_the_lan_and_settings_validate() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let base = "outlet_name=K&tax_pct=10&service_pct=5&paper_cols=32&receipt_footer=Thx&round_to=0&discount_pct=10&theme=copper";
    let bad = send(
        &a,
        "POST",
        "/owner/settings",
        Some(&owner),
        Some(&format!("{base}&printer_host=8.8.8.8:9100")),
        true,
    )
    .await;
    assert!(bad.body.contains("jaringan lokal"));
    let ok = send(
        &a,
        "POST",
        "/owner/settings",
        Some(&owner),
        Some(&format!("{base}&printer_host=192.168.1.50:9100")),
        true,
    )
    .await;
    assert!(ok.body.contains("Pengaturan disimpan"));
    // nothing listens there: test print reports a friendly failure instead of hanging or crashing
    let t = send(&a, "POST", "/owner/printer/test", Some(&owner), Some(""), true).await;
    assert_eq!(t.status, StatusCode::OK);
}

#[tokio::test]
async fn pages_have_no_inline_script_or_style() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (oid, _) = paid_order(&a, &owner).await;
    let tid: String = sqlx::query_scalar("SELECT id FROM tables")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    let token: String = sqlx::query_scalar("SELECT token FROM tables")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    for p in [
        "/owner".to_string(),
        "/owner/menu".into(),
        "/owner/tables".into(),
        format!("/owner/tables/{tid}/qr"),
        "/owner/staff".into(),
        "/owner/settings".into(),
        "/owner/audit".into(),
        "/owner/reports".into(),
        "/cashier".into(),
        "/cashier/new".into(),
        "/cashier/shift".into(),
        "/cashier/history".into(),
        "/owner/vouchers".into(),
        "/owner/expenses".into(),
        "/display".into(),
        "/owner/devices".into(),
        "/owner/wizard".into(),
        format!("/orders/{oid}"),
        format!("/orders/{oid}/receipt"),
        format!("/m/{token}"),
    ] {
        let r = send(&a, "GET", &p, Some(&owner), None, false).await;
        assert_eq!(r.status, StatusCode::OK, "{p}");
        assert!(!r.body.contains(" style="), "inline style in {p}");
        let inline = r
            .body
            .match_indices("<script")
            .filter(|(i, _)| !r.body[*i..].starts_with("<script src="))
            .count();
        assert_eq!(inline, 0, "inline script in {p}");
    }
}

#[tokio::test]
async fn customer_cannot_see_other_tables_orders() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (token, item, _) = seed(&a, &owner).await;
    send(&a, "POST", "/owner/tables", Some(&owner), Some("label=2"), true).await;
    let token2: String = sqlx::query_scalar("SELECT token FROM tables WHERE label='2'")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
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
    assert_eq!(
        send(&a, "GET", &format!("/m/{token2}/orders/{oid}"), None, None, false)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&a, "GET", &format!("/m/{token}/orders/{oid}"), None, None, false)
            .await
            .status,
        StatusCode::OK
    );
}

// ---------------------------------------------------------------------------------------------
// PRD core features
// ---------------------------------------------------------------------------------------------

async fn db_one<T: for<'r> sqlx::Decode<'r, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite> + Send + Unpin>(
    a: &App,
    sql: &'static str,
) -> T {
    sqlx::query_scalar(sql).fetch_one(&a.st.db).await.unwrap()
}

#[tokio::test]
async fn pin_login_needs_a_registered_device_and_revocation_works() {
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
    // Unregistered browser: refused even with the right PIN.
    let r = login(&a, "Budi", "1234").await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(r.body.contains("belum terdaftar"));
    // Owner password works from anywhere.
    assert_eq!(
        login(&a, "Ana", "correct+horse+battery").await.status,
        StatusCode::SEE_OTHER
    );
    let dev = register_device(&a, &owner).await;
    assert_eq!(login_with(&a, "Budi", "1234", &dev).await.status, StatusCode::SEE_OTHER);
    // Revoke -> PIN login from that device stops working.
    let id: String = db_one(&a, "SELECT id FROM devices").await;
    send(
        &a,
        "POST",
        &format!("/owner/devices/{id}/revoke"),
        Some(&owner),
        Some(""),
        true,
    )
    .await;
    assert_eq!(
        login_with(&a, "Budi", "1234", &dev).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn modifiers_are_validated_priced_and_snapshotted() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (token, item, cashier) = seed(&a, &owner).await;
    send(
        &a,
        "POST",
        &format!("/owner/items/{item}/groups"),
        Some(&owner),
        Some("name=Ukuran&max_select=1&required=1"),
        true,
    )
    .await;
    let gid: String = db_one(&a, "SELECT id FROM modifier_groups").await;
    send(
        &a,
        "POST",
        &format!("/owner/groups/{gid}/modifiers"),
        Some(&owner),
        Some("name=Regular&price=0"),
        true,
    )
    .await;
    send(
        &a,
        "POST",
        &format!("/owner/groups/{gid}/modifiers"),
        Some(&owner),
        Some("name=Large&price=5000"),
        true,
    )
    .await;
    let large: String = sqlx::query_scalar("SELECT id FROM modifiers WHERE name='Large'")
        .fetch_one(&a.st.db)
        .await
        .unwrap();

    // Required group not chosen -> rejected, nothing created.
    let r = send(
        &a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=1")),
        true,
    )
    .await;
    assert!(r.body.contains("Pilih Ukuran"), "{}", r.body);
    // A modifier id from nowhere -> rejected.
    let r = send(
        &a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=1&mod_{item}__{gid}=nope")),
        true,
    )
    .await;
    assert!(r.body.contains("tidak valid"));
    // Valid: Latte 25.000 + Large 5.000 = 30.000 x2 = 60.000 + 10% tax.
    let r = send(
        &a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=2&mod_{item}__{gid}={large}")),
        true,
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    let (name, unit): (String, i64) = sqlx::query_as("SELECT name, unit_price FROM order_items")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    assert_eq!((name.as_str(), unit), ("Latte (Large)", 30_000));
    assert_eq!(db_one::<i64>(&a, "SELECT total FROM orders").await, 66_000);
    let _ = cashier;
}

#[tokio::test]
async fn stock_decrements_blocks_oversell_and_is_restored_on_reject() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (token, item, cashier) = seed(&a, &owner).await;
    send(
        &a,
        "POST",
        &format!("/owner/items/{item}"),
        Some(&owner),
        Some("name=Latte&price=25000&stock=3"),
        true,
    )
    .await;
    let ok = send(
        &a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=2")),
        true,
    )
    .await;
    assert_eq!(ok.status, StatusCode::SEE_OTHER);
    assert_eq!(db_one::<i64>(&a, "SELECT stock FROM menu_items").await, 1);
    let over = send(
        &a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=2")),
        true,
    )
    .await;
    assert!(over.body.contains("tinggal 1"), "{}", over.body);
    // Reject gives the stock back.
    let oid: String = db_one(&a, "SELECT id FROM orders").await;
    send(
        &a,
        "POST",
        &format!("/orders/{oid}/reject"),
        Some(&cashier),
        Some("reason=iseng"),
        true,
    )
    .await;
    assert_eq!(db_one::<i64>(&a, "SELECT stock FROM menu_items").await, 3);
    // Selling the last units flips the item to sold-out automatically.
    send(
        &a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=3")),
        true,
    )
    .await;
    assert_eq!(db_one::<i64>(&a, "SELECT available FROM menu_items").await, 0);
}

async fn confirmed_order(a: &App, owner: &str, qty: i64) -> (String, String, String) {
    let (token, item, cashier) = seed(a, owner).await;
    send(
        a,
        "POST",
        "/cashier/shift/open",
        Some(&cashier),
        Some("opening=100000"),
        true,
    )
    .await;
    send(
        a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}={qty}")),
        true,
    )
    .await;
    let oid: String = db_one(a, "SELECT id FROM orders").await;
    send(
        a,
        "POST",
        &format!("/orders/{oid}/confirm"),
        Some(&cashier),
        Some(""),
        true,
    )
    .await;
    (oid, cashier, item)
}

#[tokio::test]
async fn discount_over_cashier_limit_needs_owner_approval() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (oid, cashier, _) = confirmed_order(&a, &owner, 4).await; // 100.000 subtotal
    // 5% is within the cashier limit (10%): no approval needed.
    let r = send(
        &a,
        "POST",
        &format!("/orders/{oid}/discount"),
        Some(&cashier),
        Some("mode=pct&value=5&reason=promo"),
        true,
    )
    .await;
    assert!(r.body.contains("Diskon diterapkan"), "{}", r.body);
    assert_eq!(db_one::<i64>(&a, "SELECT discount FROM orders").await, 5_000);
    // 50% needs approval; without it, refused and unchanged.
    let r = send(
        &a,
        "POST",
        &format!("/orders/{oid}/discount"),
        Some(&cashier),
        Some("mode=pct&value=50&reason=teman"),
        true,
    )
    .await;
    assert!(r.body.contains("di atas batas"), "{}", r.body);
    assert_eq!(db_one::<i64>(&a, "SELECT discount FROM orders").await, 5_000);
    // With the owner's credentials it goes through and the approver is audited.
    let r = send(
        &a,
        "POST",
        &format!("/orders/{oid}/discount"),
        Some(&cashier),
        Some("mode=pct&value=50&reason=teman&approver=Ana&secret=correct+horse+battery"),
        true,
    )
    .await;
    assert!(r.body.contains("Diskon diterapkan"), "{}", r.body);
    assert_eq!(db_one::<i64>(&a, "SELECT discount FROM orders").await, 50_000);
    // total = (100.000-50.000) * 1.10 = 55.000
    assert_eq!(db_one::<i64>(&a, "SELECT total FROM orders").await, 55_000);
    let approver: Option<String> =
        sqlx::query_scalar("SELECT approver_id FROM audit_log WHERE action='order.discount' ORDER BY id DESC LIMIT 1")
            .fetch_one(&a.st.db)
            .await
            .unwrap();
    assert!(approver.is_some());
}

#[tokio::test]
async fn item_void_recalculates_and_cancels_empty_orders() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (oid, cashier, _) = confirmed_order(&a, &owner, 2).await;
    let item_id: String = db_one(&a, "SELECT id FROM order_items").await;
    let r = send(
        &a,
        "POST",
        &format!("/orders/{oid}/items/{item_id}/void"),
        Some(&cashier),
        Some("reason="),
        true,
    )
    .await;
    assert!(r.body.contains("Alasan wajib"));
    send(
        &a,
        "POST",
        &format!("/orders/{oid}/items/{item_id}/void"),
        Some(&cashier),
        Some("reason=salah+pesan"),
        true,
    )
    .await;
    assert_eq!(db_one::<i64>(&a, "SELECT total FROM orders").await, 0);
    assert_eq!(db_one::<String>(&a, "SELECT status FROM orders").await, "cancelled");
}

#[tokio::test]
async fn refund_is_partial_capped_approved_and_reduces_the_drawer() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (oid, cashier, _) = confirmed_order(&a, &owner, 2).await; // 55.000
    send(
        &a,
        "POST",
        &format!("/orders/{oid}/pay"),
        Some(&cashier),
        Some("method=cash&tendered=55000"),
        true,
    )
    .await;
    // Cashiers cannot refund by default...
    let denied = send(
        &a,
        "POST",
        &format!("/orders/{oid}/refund"),
        Some(&cashier),
        Some("amount=1000&reason=x&approver=Ana&secret=correct+horse+battery"),
        true,
    )
    .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    // ...until the owner grants the permission to this user.
    let uid: String = db_one(&a, "SELECT id FROM users WHERE name='Budi'").await;
    send(
        &a,
        "POST",
        &format!("/owner/staff/{uid}/perms"),
        Some(&owner),
        Some("perm_refund=1&perm_discount=1&perm_cash_move=1&perm_history=1&perm_sold_out=1"),
        true,
    )
    .await;
    let bad = send(
        &a,
        "POST",
        &format!("/orders/{oid}/refund"),
        Some(&cashier),
        Some("amount=10000&reason=x&approver=Ana&secret=wrong"),
        true,
    )
    .await;
    assert!(bad.body.contains("salah atau terkunci"));
    let ok = send(
        &a,
        "POST",
        &format!("/orders/{oid}/refund"),
        Some(&cashier),
        Some("amount=20000&reason=kopi+dingin&approver=Ana&secret=correct+horse+battery"),
        true,
    )
    .await;
    assert!(ok.body.contains("Refund dicatat"), "{}", ok.body);
    let over = send(
        &a,
        "POST",
        &format!("/orders/{oid}/refund"),
        Some(&cashier),
        Some("amount=40000&reason=x&approver=Ana&secret=correct+horse+battery"),
        true,
    )
    .await;
    assert!(over.body.contains("melebihi"));
    // Refund documents are immutable, and the cash left the drawer.
    assert!(
        sqlx::query("UPDATE refunds SET amount = 1")
            .execute(&a.st.db)
            .await
            .is_err()
    );
    assert_eq!(
        db_one::<i64>(&a, "SELECT amount FROM cash_movements WHERE kind='out'").await,
        20_000
    );
    // Blind close: expected = 100.000 + 55.000 - 20.000 = 135.000
    let z = send(
        &a,
        "POST",
        "/cashier/shift/close",
        Some(&cashier),
        Some("counted=135000"),
        true,
    )
    .await;
    assert!(z.body.contains("Rp135.000"), "{}", z.body);
    // Net sales in the report subtract the refund.
    let rep = send(&a, "GET", "/owner/reports", Some(&owner), None, false).await;
    assert!(rep.body.contains("Rp35.000"), "{}", rep.body);
}

#[tokio::test]
async fn cash_in_out_is_recorded_and_counted_in_the_drawer() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (_, cashier, _) = confirmed_order(&a, &owner, 1).await;
    let r = send(
        &a,
        "POST",
        "/cashier/shift/cash",
        Some(&cashier),
        Some("kind=in&amount=50000&reason=tambah+receh"),
        true,
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let bad = send(
        &a,
        "POST",
        "/cashier/shift/cash",
        Some(&cashier),
        Some("kind=out&amount=0&reason=x"),
        true,
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    send(
        &a,
        "POST",
        "/cashier/shift/cash",
        Some(&cashier),
        Some("kind=out&amount=10000&reason=beli+es"),
        true,
    )
    .await;
    let z = send(
        &a,
        "POST",
        "/cashier/shift/close",
        Some(&cashier),
        Some("counted=140000"),
        true,
    )
    .await;
    // 100.000 + 50.000 - 10.000 = 140.000 -> no difference
    assert!(z.body.contains("Rp140.000") && z.body.contains("Rp0"), "{}", z.body);
}

#[tokio::test]
async fn permission_overrides_grant_and_revoke_per_user() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (_, _, cashier) = (0, 0, seed(&a, &owner).await.2);
    // Default: a cashier cannot see reports.
    assert_eq!(
        send(&a, "GET", "/owner/reports", Some(&cashier), None, false)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let uid: String = db_one(&a, "SELECT id FROM users WHERE name='Budi'").await;
    // Grant reports, revoke history.
    send(
        &a,
        "POST",
        &format!("/owner/staff/{uid}/perms"),
        Some(&owner),
        Some("perm_reports=1&perm_discount=1&perm_cash_move=1&perm_sold_out=1"),
        true,
    )
    .await;
    assert_eq!(
        send(&a, "GET", "/owner/reports", Some(&cashier), None, false)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        send(&a, "GET", "/cashier/history", Some(&cashier), None, false)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    // Overrides are stored only where they differ from the role default.
    assert_eq!(
        db_one::<i64>(&a, "SELECT COUNT(*) FROM user_permission_overrides").await,
        2
    );
}

#[tokio::test]
async fn flow_modes_hours_rounding_and_history() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (token, item, cashier) = seed(&a, &owner).await;
    // Cashier-only flow hides the kitchen screen.
    assert_eq!(
        send(&a, "GET", "/kitchen", Some(&owner), None, false).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let base = "outlet_name=K&tax_pct=10&service_pct=0&paper_cols=32&receipt_footer=Thx&discount_pct=10&theme=copper&qr_ordering=1";
    let ok = send(
        &a,
        "POST",
        "/owner/settings",
        Some(&owner),
        Some(&format!(
            "{base}&round_to=500&kitchen_mode=1&open_from=00:00&open_to=00:01"
        )),
        true,
    )
    .await;
    assert!(ok.body.contains("Pengaturan disimpan"), "{}", ok.body);
    assert_eq!(
        send(&a, "GET", "/kitchen", Some(&owner), None, false).await.status,
        StatusCode::OK
    );
    // Closed hours: customers see the banner and cannot order.
    let page = send(&a, "GET", &format!("/m/{token}"), None, None, false).await;
    assert!(page.body.contains("Kafe sedang tutup"));
    let r = send(
        &a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=1")),
        true,
    )
    .await;
    assert!(r.body.contains("sedang tutup"));
    // Reopen, rounding to Rp500 is its own recorded line: 25.000 + 10% = 27.500 (already round) -> use 3 items 82.500 stays; check 1 item.
    send(
        &a,
        "POST",
        "/owner/settings",
        Some(&owner),
        Some(&format!("{base}&round_to=1000&open_from=&open_to=")),
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
    let (total, rounding): (i64, i64) = sqlx::query_as("SELECT total, rounding FROM orders")
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    assert_eq!((total, rounding), (28_000, 500)); // 27.500 -> 28.000
    // History search finds it by number and by table.
    let h = send(&a, "GET", "/cashier/history?q=1", Some(&cashier), None, false).await;
    assert!(h.body.contains("Rp28.000"));
    let none = send(&a, "GET", "/cashier/history?q=zzz", Some(&cashier), None, false).await;
    assert!(none.body.contains("Tidak ada pesanan"));
}

#[tokio::test]
async fn csv_import_creates_and_updates_menu_safely() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let csv = "kategori,nama,harga,stok%0AKopi,Americano,18000%0AKopi,Latte,25000,40%0AMakanan,Roti,12000";
    let r = send(
        &a,
        "POST",
        "/owner/menu/import",
        Some(&owner),
        Some(&format!("csv={csv}")),
        true,
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    assert_eq!(db_one::<i64>(&a, "SELECT COUNT(*) FROM menu_items").await, 3);
    // Re-import updates price instead of duplicating.
    send(
        &a,
        "POST",
        "/owner/menu/import",
        Some(&owner),
        Some("csv=Kopi,Americano,19000"),
        true,
    )
    .await;
    assert_eq!(db_one::<i64>(&a, "SELECT COUNT(*) FROM menu_items").await, 3);
    assert_eq!(
        db_one::<i64>(&a, "SELECT price FROM menu_items WHERE name='Americano'").await,
        19_000
    );
    // Bad rows are rejected as a whole (atomic).
    let bad = send(
        &a,
        "POST",
        "/owner/menu/import",
        Some(&owner),
        Some("csv=Kopi,Mocha,30000%0AKopi,Broken,abc"),
        true,
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(db_one::<i64>(&a, "SELECT COUNT(*) FROM menu_items").await, 3);
}

#[tokio::test]
async fn move_table_and_sold_out_toggle_by_cashier() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (oid, cashier, item) = confirmed_order(&a, &owner, 1).await;
    send(&a, "POST", "/owner/tables", Some(&owner), Some("label=2"), true).await;
    let t2: String = db_one(&a, "SELECT id FROM tables WHERE label='2'").await;
    send(
        &a,
        "POST",
        &format!("/orders/{oid}/move"),
        Some(&cashier),
        Some(&format!("table_id={t2}")),
        true,
    )
    .await;
    assert_eq!(db_one::<String>(&a, "SELECT table_id FROM orders").await, t2);
    send(
        &a,
        "POST",
        &format!("/menu/{item}/available"),
        Some(&cashier),
        Some(""),
        true,
    )
    .await;
    assert_eq!(db_one::<i64>(&a, "SELECT available FROM menu_items").await, 0);
}

#[tokio::test]
async fn backup_verify_checks_integrity() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let b = send(&a, "POST", "/owner/backup", Some(&owner), Some(""), true).await;
    let path = std::env::temp_dir().join(format!("verify-{}.db", cuppa::rand_id::id()));
    std::fs::write(&path, b.body.as_bytes()).unwrap();
    // `body` was lossy-decoded for text, so verify a fresh VACUUM INTO snapshot instead.
    let snap = std::env::temp_dir().join(format!("snap-{}.db", cuppa::rand_id::id()));
    cuppa::backup::snapshot(&a.st, &snap).await.unwrap();
    cuppa::backup::verify(snap.to_str().unwrap()).await.unwrap();
    std::fs::write(&path, b"not a database").unwrap();
    assert!(cuppa::backup::verify(path.to_str().unwrap()).await.is_err());
}

// ---------------------------------------------------------------------------------------------
// Extended features: split payment, vouchers, digital receipt, calls, rating, display, expenses
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn split_and_mixed_payment_locks_only_when_fully_paid() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (oid, cashier, _) = confirmed_order(&a, &owner, 2).await; // 55.000
    let p1 = send(
        &a,
        "POST",
        &format!("/orders/{oid}/pay"),
        Some(&cashier),
        Some("method=cash&amount=20000&tendered=20000"),
        true,
    )
    .await;
    assert!(p1.body.contains("Sisa Rp35.000"), "{}", p1.body);
    assert_eq!(db_one::<String>(&a, "SELECT status FROM orders").await, "confirmed");
    // Frozen after a partial payment: no discount, no item void.
    let d = send(
        &a,
        "POST",
        &format!("/orders/{oid}/discount"),
        Some(&cashier),
        Some("mode=pct&value=5&reason=x"),
        true,
    )
    .await;
    assert_eq!(d.status, StatusCode::UNPROCESSABLE_ENTITY);
    // Over-paying is refused.
    let over = send(
        &a,
        "POST",
        &format!("/orders/{oid}/pay"),
        Some(&cashier),
        Some("method=qris&amount=40000&reference=R1"),
        true,
    )
    .await;
    assert!(over.body.contains("sisa tagihan"), "{}", over.body);
    // Second payment, different method, settles it and locks the order.
    let p2 = send(
        &a,
        "POST",
        &format!("/orders/{oid}/pay"),
        Some(&cashier),
        Some("method=qris&reference=R1"),
        true,
    )
    .await;
    assert_eq!(p2.status, StatusCode::SEE_OTHER);
    assert_eq!(db_one::<String>(&a, "SELECT status FROM orders").await, "paid");
    assert_eq!(db_one::<i64>(&a, "SELECT COUNT(*) FROM payments").await, 2);
    assert!(
        sqlx::query("UPDATE orders SET total = 1")
            .execute(&a.st.db)
            .await
            .is_err()
    );
    // Receipt shows both payments; only the cash part counts toward the drawer.
    let rc = send(
        &a,
        "GET",
        &format!("/orders/{oid}/receipt"),
        Some(&cashier),
        None,
        false,
    )
    .await;
    assert!(rc.body.contains("Bayar (Tunai)") && rc.body.contains("Bayar (QRIS)"));
    let z = send(
        &a,
        "POST",
        "/cashier/shift/close",
        Some(&cashier),
        Some("counted=120000"),
        true,
    )
    .await;
    assert!(z.body.contains("Rp120.000"), "{}", z.body); // 100.000 + 20.000 cash
}

#[tokio::test]
async fn vouchers_validate_limits_and_apply_once() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let r = send(
        &a,
        "POST",
        "/owner/vouchers",
        Some(&owner),
        Some("code=hemat10&kind=pct&value=10&min_subtotal=40000&max_uses=1"),
        true,
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    let bad = send(
        &a,
        "POST",
        "/owner/vouchers",
        Some(&owner),
        Some("code=x&kind=pct&value=150"),
        true,
    )
    .await;
    assert!(bad.body.contains("Kode 3"));
    let (oid, cashier, _) = confirmed_order(&a, &owner, 2).await; // subtotal 50.000
    let unknown = send(
        &a,
        "POST",
        &format!("/orders/{oid}/voucher"),
        Some(&cashier),
        Some("code=NOPE"),
        true,
    )
    .await;
    assert!(unknown.body.contains("tidak dikenal"));
    let ok = send(
        &a,
        "POST",
        &format!("/orders/{oid}/voucher"),
        Some(&cashier),
        Some("code=HEMAT10"),
        true,
    )
    .await;
    assert!(ok.body.contains("Voucher diterapkan"), "{}", ok.body);
    assert_eq!(db_one::<i64>(&a, "SELECT discount FROM orders").await, 5_000);
    assert_eq!(db_one::<i64>(&a, "SELECT used FROM vouchers").await, 1);
    // max_uses = 1 -> a second order cannot use it.
    let (token, item) = (
        db_one::<String>(&a, "SELECT token FROM tables").await,
        db_one::<String>(&a, "SELECT id FROM menu_items").await,
    );
    send(
        &a,
        "POST",
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=2")),
        true,
    )
    .await;
    let o2: String = sqlx::query_scalar("SELECT id FROM orders WHERE id != ? LIMIT 1")
        .bind(&oid)
        .fetch_one(&a.st.db)
        .await
        .unwrap();
    send(
        &a,
        "POST",
        &format!("/orders/{o2}/confirm"),
        Some(&cashier),
        Some(""),
        true,
    )
    .await;
    let again = send(
        &a,
        "POST",
        &format!("/orders/{o2}/voucher"),
        Some(&cashier),
        Some("code=HEMAT10"),
        true,
    )
    .await;
    assert!(again.body.contains("habis dipakai"), "{}", again.body);
}

#[tokio::test]
async fn digital_receipt_link_is_public_unguessable_and_read_only() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (oid, cashier, _) = confirmed_order(&a, &owner, 2).await;
    // Not available before payment.
    assert_eq!(
        send(&a, "GET", "/r/00000000000000000000000000000000", None, None, false)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    send(
        &a,
        "POST",
        &format!("/orders/{oid}/pay"),
        Some(&cashier),
        Some("method=cash&tendered=60000"),
        true,
    )
    .await;
    let token: String = db_one(&a, "SELECT receipt_token FROM orders").await;
    assert_eq!(token.len(), 32);
    let r = send(&a, "GET", &format!("/r/{token}"), None, None, false).await; // no cookie
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("Rp55.000") && !r.body.contains("Budi"));
    let page = send(
        &a,
        "GET",
        &format!("/orders/{oid}/receipt"),
        Some(&cashier),
        None,
        false,
    )
    .await;
    assert!(page.body.contains("wa.me") && page.body.contains(&token));
    assert_eq!(
        send(&a, "GET", "/r/not-a-token", None, None, false).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn table_calls_rating_bill_and_display() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (oid, cashier, _) = confirmed_order(&a, &owner, 1).await;
    let token: String = db_one(&a, "SELECT token FROM tables").await;
    // Customer asks for the bill (twice: deduplicated) and the cashier sees one call.
    send(&a, "POST", &format!("/m/{token}/call"), None, Some("kind=bill"), true).await;
    send(&a, "POST", &format!("/m/{token}/call"), None, Some("kind=bill"), true).await;
    assert_eq!(db_one::<i64>(&a, "SELECT COUNT(*) FROM table_calls").await, 1);
    let q = send(&a, "GET", "/cashier/queue", Some(&cashier), None, false).await;
    assert!(q.body.contains("minta bill"));
    let cid: String = db_one(&a, "SELECT id FROM table_calls").await;
    send(
        &a,
        "POST",
        &format!("/calls/{cid}/done"),
        Some(&cashier),
        Some(""),
        true,
    )
    .await;
    assert!(
        !send(&a, "GET", "/cashier/queue", Some(&cashier), None, false)
            .await
            .body
            .contains("minta bill")
    );
    assert_eq!(
        send(&a, "POST", &format!("/m/{token}/call"), None, Some("kind=evil"), true)
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Bill page lists the table's orders.
    let bill = send(&a, "GET", &format!("/m/{token}/bill"), None, None, false).await;
    assert!(bill.body.contains("Minta bill") && bill.body.contains("Latte"));
    // Rating only after payment, once per order.
    let early = send(
        &a,
        "POST",
        &format!("/m/{token}/orders/{oid}/rating"),
        None,
        Some("stars=5"),
        true,
    )
    .await;
    assert_eq!(early.status, StatusCode::NOT_FOUND);
    send(
        &a,
        "POST",
        &format!("/orders/{oid}/pay"),
        Some(&cashier),
        Some("method=cash&tendered=30000"),
        true,
    )
    .await;
    send(
        &a,
        "POST",
        &format!("/m/{token}/orders/{oid}/rating"),
        None,
        Some("stars=5&comment=enak"),
        true,
    )
    .await;
    send(
        &a,
        "POST",
        &format!("/m/{token}/orders/{oid}/rating"),
        None,
        Some("stars=1"),
        true,
    )
    .await;
    assert_eq!(db_one::<i64>(&a, "SELECT stars FROM order_ratings").await, 5);
    assert_eq!(
        send(
            &a,
            "POST",
            &format!("/m/{token}/orders/{oid}/rating"),
            None,
            Some("stars=9"),
            true
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Queue display shows numbers only (public).
    let d = send(&a, "GET", "/display/part", None, None, false).await;
    assert_eq!(d.status, StatusCode::OK);
    assert!(d.body.contains("num-big") && !d.body.contains("Latte"));
}

#[tokio::test]
async fn expenses_are_owner_only_and_validated() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (_, _, cashier) = seed(&a, &owner).await;
    assert_eq!(
        send(&a, "GET", "/owner/expenses", Some(&cashier), None, false)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &a,
            "POST",
            "/owner/expenses",
            Some(&cashier),
            Some("day=2026-11-24&category=x&amount=1&kind=fixed"),
            true
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let bad = send(
        &a,
        "POST",
        "/owner/expenses",
        Some(&owner),
        Some("day=bad&category=Sewa&amount=1000&kind=fixed"),
        true,
    )
    .await;
    assert!(bad.body.contains("wajib diisi"));
    let ok = send(
        &a,
        "POST",
        "/owner/expenses",
        Some(&owner),
        Some("day=2026-11-24&category=Sewa&amount=3000000&kind=fixed"),
        true,
    )
    .await;
    assert_eq!(ok.status, StatusCode::SEE_OTHER);
    assert_eq!(db_one::<i64>(&a, "SELECT amount FROM expenses").await, 3_000_000);
}

#[tokio::test]
async fn profit_report_uses_cost_snapshot_and_expenses() {
    let a = app().await;
    let owner = setup_owner(&a).await;
    let (token, item, cashier) = seed(&a, &owner).await;
    // Latte price 25.000, cost 10.000; the cost is snapshotted per order line.
    send(
        &a,
        "POST",
        &format!("/owner/items/{item}"),
        Some(&owner),
        Some("name=Latte&price=25000&cost=10000"),
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
        &format!("/m/{token}/orders"),
        None,
        Some(&format!("qty_{item}=2")),
        true,
    )
    .await;
    let oid: String = db_one(&a, "SELECT id FROM orders").await;
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
        Some("method=cash&tendered=60000"),
        true,
    )
    .await;
    // Raising the cost later does not rewrite the past order.
    send(
        &a,
        "POST",
        &format!("/owner/items/{item}"),
        Some(&owner),
        Some("name=Latte&price=25000&cost=99999"),
        true,
    )
    .await;
    assert_eq!(db_one::<i64>(&a, "SELECT unit_cost FROM order_items").await, 10_000);
    let today = db_one::<String>(&a, "SELECT day FROM orders").await;
    send(
        &a,
        "POST",
        "/owner/expenses",
        Some(&owner),
        Some(&format!("day={today}&category=Listrik&amount=5000&kind=variable")),
        true,
    )
    .await;
    let rep = send(&a, "GET", "/owner/reports", Some(&owner), None, false).await;
    // revenue ex-tax 50.000 - HPP 20.000 = 30.000 gross; - 5.000 expenses = 25.000 net
    assert!(
        rep.body.contains("Rp20.000") && rep.body.contains("Rp30.000") && rep.body.contains("Rp25.000"),
        "{}",
        rep.body
    );
}
