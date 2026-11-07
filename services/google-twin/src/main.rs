use google_twin::{app, AppState, Config};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info")),
        )
        .init();

    let config = Config::from_env();
    let state = Arc::new(AppState::new(config.clone()));
    {
        let st = state.read();
        println!(
            "google-twin: seeded workspace {}",
            google_twin::seed::DOMAIN
        );
        for u in st.users.values() {
            println!(
                "  {:<11} {:<18} {}",
                u.name,
                u.display_name.as_deref().unwrap_or(""),
                u.email.as_deref().unwrap_or("")
            );
        }
        println!(
            "  {} spaces, {} messages, {} reactions, {} attachments, {} calendar events",
            st.spaces.len(),
            st.messages.len(),
            st.reactions.len(),
            st.blobs.len(),
            st.cal_events.len()
        );
    }
    println!("  events: {}", state.pusher.describe());
    println!("  sign-in page: {}/o/oauth2/v2/auth?client_id=prism-dev&redirect_uri=http://127.0.0.1:1/callback&response_type=code&state=x", config.public_base);
    println!(
        "  script token: curl -X POST '{}/twin/token?user=users/1002'",
        config.public_base
    );

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port)).await?;
    println!(
        "listening on http://0.0.0.0:{} (public base {})",
        config.port, config.public_base
    );
    tracing::info!(port = config.port, public_base = %config.public_base, "google-twin up");
    axum::serve(listener, app(state)).await?;
    Ok(())
}
