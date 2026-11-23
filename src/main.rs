// SPDX-License-Identifier: AGPL-3.0-or-later
use cuppa::{AppState, config::Config, router, spawn_housekeeping};
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cuppa=info,tower_http=warn".into()),
        )
        .init();
    // `cuppa verify-backup <file>`: integrity check + row counts, for the monthly restore test (FR-37).
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("verify-backup") {
        let path = args
            .get(2)
            .ok_or_else(|| anyhow::anyhow!("usage: cuppa verify-backup <file.db>"))?;
        return cuppa::backup::verify(path).await;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("cuppa {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let cfg = Config::from_env()?;
    let bind = cfg.bind;
    let st = AppState::new(cfg).await?;
    if let Some(code) = st.setup_code.lock().unwrap().clone() {
        tracing::warn!(
            "FIRST RUN — open {}/setup and enter setup code: {code}",
            st.cfg.public_url
        );
    }
    spawn_housekeeping(st.clone());
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!("cuppa listening on http://{bind}");
    axum::serve(listener, router(st).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
