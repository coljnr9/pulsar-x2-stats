use crate::state::SharedState;
use axum::{
    Router,
    extract::State,
    response::{Html, IntoResponse, Json},
    routing::get,
};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tower_http::trace::TraceLayer;

pub async fn serve(
    state: SharedState,
    bind_addr: String,
    shutdown_rx: oneshot::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = Router::new()
        .route("/state", get(state_json))
        .route("/waybar", get(waybar_json))
        .route("/", get(serve_dashboard))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = bind_addr.parse()?;
    let listener = TcpListener::bind(addr).await?;

    tracing::info!("HTTP server listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
            tracing::info!("HTTP server shutting down");
        })
        .await?;

    Ok(())
}

async fn state_json(State(state): State<SharedState>) -> impl IntoResponse {
    let current_state = state.read().clone();
    Json(current_state)
}

async fn waybar_json(State(state): State<SharedState>) -> impl IntoResponse {
    let current_state = state.read().clone();
    let waybar_json = crate::format::waybar(&current_state);
    Json(waybar_json)
}

async fn serve_dashboard() -> Html<&'static str> {
    Html(include_str!("../assets/dashboard.html"))
}
