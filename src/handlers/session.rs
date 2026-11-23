// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::{
    AppState,
    audit::{self, Entry},
    auth::{self, CredError, Role, Staff},
    clock,
    error::{AppError, AppResult},
    handlers::clean,
    views::{Ctx, LoginPage, SetupPage, render},
};
use axum::{
    Form,
    extract::{ConnectInfo, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::{net::SocketAddr, time::Duration};

async fn user_count(st: &AppState) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&st.db)
        .await?)
}

pub async fn root(State(st): State<AppState>, staff: Result<Staff, AppError>) -> AppResult<Redirect> {
    if user_count(&st).await? == 0 {
        return Ok(Redirect::to("/setup"));
    }
    Ok(match staff {
        Ok(s) => Redirect::to(s.role.home()),
        Err(_) => Redirect::to("/login"),
    })
}

async fn login_code(st: &AppState) -> AppResult<Response> {
    Ok((
        axum::http::StatusCode::UNAUTHORIZED,
        render(LoginPage {
            ctx: Ctx::guest(st).await?,
            error: Some("Masukkan kode 6 digit dari aplikasi autentikator.".into()),
            need_code: true,
        })?,
    )
        .into_response())
}

async fn login_error(st: &AppState, msg: &str) -> AppResult<Response> {
    Ok((
        axum::http::StatusCode::UNAUTHORIZED,
        render(LoginPage {
            ctx: Ctx::guest(st).await?,
            error: Some(msg.into()),
            need_code: false,
        })?,
    )
        .into_response())
}

async fn setup_error(st: &AppState, msg: &str) -> AppResult<Response> {
    Ok((
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        render(SetupPage {
            ctx: Ctx::guest(st).await?,
            error: Some(msg.into()),
        })?,
    )
        .into_response())
}

pub async fn login_page(State(st): State<AppState>) -> AppResult<Response> {
    if user_count(&st).await? == 0 {
        return Ok(Redirect::to("/setup").into_response());
    }
    Ok(render(LoginPage {
        ctx: Ctx::guest(&st).await?,
        error: None,
        need_code: false,
    })?
    .into_response())
}

#[derive(Deserialize)]
pub struct LoginForm {
    name: String,
    secret: String,
    code: Option<String>,
}

pub async fn login(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(f): Form<LoginForm>,
) -> AppResult<Response> {
    let ip = crate::security::client_ip(&st, &headers, peer);
    if !st.limiter.allow(&format!("login:{ip}"), 10, Duration::from_secs(60)) {
        return Err(AppError::TooMany);
    }
    match auth::check_credentials(&st.db, &f.name, &f.secret, auth::Second::Code(f.code.as_deref())).await {
        Ok(staff) => {
            if staff.role != Role::Owner {
                let required: i64 = sqlx::query_scalar("SELECT require_device FROM outlet_settings WHERE id = 1")
                    .fetch_one(&st.db)
                    .await?;
                if required == 1 && !super::extras::device_ok(&st, &headers).await {
                    return login_error(&st, "Perangkat ini belum terdaftar. Minta owner masuk di perangkat ini lalu daftarkan di menu Perangkat.").await;
                }
            }
            let token = auth::create_session(&st.db, &staff.id).await?;
            let mut c = st.db.acquire().await?;
            audit::record(
                &mut c,
                Entry::new(Some(&staff.id), "auth.login", "user", Some(&staff.id)),
            )
            .await?;
            Ok((
                [(header::SET_COOKIE, auth::session_cookie(&token, st.cfg.cookie_secure))],
                Redirect::to(staff.role.home()),
            )
                .into_response())
        }
        Err(CredError::Locked) => login_error(&st, "Akun dikunci sementara. Coba lagi dalam beberapa menit.").await,
        Err(CredError::NeedCode) => login_code(&st).await,
        Err(CredError::Invalid) => login_error(&st, "Nama, PIN, atau kode salah.").await,
    }
}

pub async fn logout(State(st): State<AppState>, headers: HeaderMap) -> AppResult<Response> {
    if let Some(t) = auth::cookie_value(&headers, auth::COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(auth::hash_token(t))
            .execute(&st.db)
            .await?;
    }
    Ok((
        [(header::SET_COOKIE, auth::clear_cookie(st.cfg.cookie_secure))],
        Redirect::to("/login"),
    )
        .into_response())
}

pub async fn setup_page(State(st): State<AppState>) -> AppResult<Response> {
    if user_count(&st).await? > 0 {
        return Ok(Redirect::to("/login").into_response());
    }
    Ok(render(SetupPage {
        ctx: Ctx::guest(&st).await?,
        error: None,
    })?
    .into_response())
}

#[derive(Deserialize)]
pub struct SetupForm {
    code: String,
    outlet: String,
    name: String,
    secret: String,
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub async fn setup(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(f): Form<SetupForm>,
) -> AppResult<Response> {
    let ip = crate::security::client_ip(&st, &headers, peer);
    if !st.limiter.allow(&format!("setup:{ip}"), 5, Duration::from_secs(60)) {
        return Err(AppError::TooMany);
    }
    let code = st.setup_code.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let Some(code) = code else {
        return Ok(Redirect::to("/login").into_response());
    };
    if !ct_eq(f.code.trim(), &code) {
        return setup_error(&st, "Kode pemasangan salah.").await;
    }
    if let Err(e) = auth::validate_secret(Role::Owner, &f.secret) {
        return setup_error(&st, &e).await;
    }
    let (name, outlet) = (clean(&f.name, 60), clean(&f.outlet, 60));
    if name.is_empty() || outlet.is_empty() {
        return setup_error(&st, "Nama wajib diisi.").await;
    }
    let hash = auth::hash_secret(f.secret).await?;
    let id = crate::rand_id::id();
    let mut tx = crate::db::begin_write(&st.db).await?;
    // Atomic "only if no user exists" guard: the setup endpoint can create exactly one owner, ever.
    let inserted = sqlx::query(
        "INSERT INTO users (id, name, role, pin_hash, created_at) SELECT ?,?,'owner',?,? WHERE NOT EXISTS (SELECT 1 FROM users)",
    )
    .bind(&id).bind(&name).bind(&hash).bind(clock::now())
    .execute(&mut *tx).await?;
    if inserted.rows_affected() == 0 {
        return Ok(Redirect::to("/login").into_response());
    }
    sqlx::query("UPDATE outlet_settings SET outlet_name = ? WHERE id = 1")
        .bind(&outlet)
        .execute(&mut *tx)
        .await?;
    audit::record(&mut tx, Entry::new(Some(&id), "setup.owner_created", "user", Some(&id))).await?;
    tx.commit().await?;
    *st.setup_code.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let token = auth::create_session(&st.db, &id).await?;
    Ok((
        [(header::SET_COOKIE, auth::session_cookie(&token, st.cfg.cookie_secure))],
        Redirect::to("/owner"),
    )
        .into_response())
}
