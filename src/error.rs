// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};

#[derive(Debug)]
pub enum AppError {
    /// Not signed in: redirect to the login page.
    Login,
    Forbidden,
    NotFound,
    /// Safe-to-show message for the user (validation, business rule).
    Invalid(String),
    TooMany,
    /// Anything else. Logged, never shown.
    Internal(anyhow::Error),
}

pub type AppResult<T> = Result<T, AppError>;

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        AppError::Internal(e.into())
    }
}

fn page(status: StatusCode, title: &str, msg: &str) -> Response {
    let body = format!(
        "<!doctype html><html lang=\"id\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{title} · Cuppa</title><link rel=\"stylesheet\" href=\"/static/css/tokens.css\"><link rel=\"stylesheet\" href=\"/static/css/app.css\"><link rel=\"stylesheet\" href=\"/static/css/shell.css\"></head>\
         <body><div class=\"login\"><div class=\"panel\"><h1>{title}</h1><p class=\"muted\">{msg}</p>\
         <a class=\"btn\" href=\"/\">Kembali</a></div></div></body></html>"
    );
    (status, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Login => Redirect::to("/login").into_response(),
            AppError::Forbidden => page(
                StatusCode::FORBIDDEN,
                "Tidak diizinkan",
                "Akun ini tidak punya izin untuk tindakan tersebut.",
            ),
            AppError::NotFound => page(
                StatusCode::NOT_FOUND,
                "Tidak ditemukan",
                "Halaman atau data yang dicari tidak ada.",
            ),
            AppError::Invalid(m) => page(StatusCode::UNPROCESSABLE_ENTITY, "Permintaan tidak valid", &esc(&m)),
            AppError::TooMany => page(
                StatusCode::TOO_MANY_REQUESTS,
                "Terlalu banyak percobaan",
                "Tunggu sebentar lalu coba lagi.",
            ),
            AppError::Internal(e) => {
                tracing::error!(error = ?e, "internal error");
                page(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Terjadi kesalahan",
                    "Kesalahan di server. Tim sudah dicatat di log.",
                )
            }
        }
    }
}
