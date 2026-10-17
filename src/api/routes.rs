use axum::{
    extract::{Query, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::chains::evm::BaseSwapOrder;
use crate::swap::engine::SwapEngine;
use crate::swap::orderbook::Orderbook;
use crate::tru_p2p::BlockScanner;
use crate::tui::TuiManager;
use crate::wallet::keystore::EncryptedVault;
use crate::wallet::vault::TruNodeClient;

use super::handlers::{
    get_estimate_fee, get_mempool, get_swaps_ledger, get_vault_export, get_vault_status,
    post_vault_withdraw,
};
use super::websocket::ws_handler;

const HTML_CONTENT: &str = include_str!("../ui/index.html");
const EXPLORER_HTML: &str = include_str!("../ui/explorer.html");
const DOCS_HTML: &str = include_str!("../ui/docs.html");
const CSS_VARIABLES: &str = include_str!("../ui/css/variables.css");
const CSS_HEADER: &str = include_str!("../ui/css/header.css");
const CSS_CARDS: &str = include_str!("../ui/css/cards.css");
const CSS_MODALS: &str = include_str!("../ui/css/modals.css");

const JS_CONFIG: &str = include_str!("../ui/js/config.js");
const JS_SOCKET: &str = include_str!("../ui/js/socket.js");
const JS_ORDERBOOK: &str = include_str!("../ui/js/orderbook.js");
const JS_SWAP: &str = include_str!("../ui/js/swap.js");
const JS_VAULT: &str = include_str!("../ui/js/vault.js");
const JS_APP: &str = include_str!("../ui/js/app.js");
const JS_QRCODE: &str = include_str!("../ui/js/qrcode.min.js");
const JS_JSQR: &str = include_str!("../ui/js/jsqr.min.js");
const JS_EXPLORER: &str = include_str!("../ui/js/explorer.js");

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<crate::config::Config>,
    pub session_token: String,
    pub orderbook: Arc<Orderbook>,
    pub tru_client: Arc<TruNodeClient>,
    pub tx_events: broadcast::Sender<BaseSwapOrder>,
    pub tui: Arc<TuiManager>,
    pub vault: Arc<tokio::sync::Mutex<EncryptedVault>>,
    pub swap_engine: Arc<SwapEngine>,
    pub block_scanner: Arc<BlockScanner>,
    pub ui_events: broadcast::Sender<serde_json::Value>,
    pub relay_client: Arc<crate::relay::client::RelayClientHandle>,
}

#[derive(Deserialize)]
pub struct AuthQuery {
    pub token: Option<String>,
}

#[derive(Deserialize)]
pub struct OpenBrowserQuery {
    pub url: Option<String>,
}

#[derive(Deserialize)]
pub struct OpenBrowserPayload {
    pub url: Option<String>,
}

#[derive(Deserialize)]
pub struct WithdrawRequest {
    pub destination: String,
    pub amount_tru: f64,
}

#[derive(Deserialize)]
pub struct FeeEstimateQuery {
    pub amount_tru: Option<f64>,
}

#[derive(Deserialize)]
pub struct LockHtlcRequest {
    pub amount_tru: f64,
    pub claim_pubkey_hex: String,
    pub timelock: u32,
    pub hash160_hex: String,
}

#[derive(Deserialize)]
pub struct ClaimHtlcRequest {
    pub htlc_txid: String,
    pub htlc_vout: u32,
    pub htlc_atoms: u64,
    pub htlc_script_hex: String,
    pub secret_hex: String,
    pub claimer_priv_key_hex: String,
    pub destination_address: String,
}

#[derive(Deserialize)]
pub struct CreateOrderRequest {
    pub amount_tru: f64,
    pub amount_usdc: f64,
    pub timelock_hours: Option<u16>,
    pub maker_base_address: Option<String>,
    pub side: Option<String>,
}

#[derive(Deserialize)]
pub struct LockBaseHtlcRequest {
    pub hash160: String,
    pub amount_usdc: f64,
    pub beneficiary: Option<String>,
    pub timelock: Option<u32>,
}

#[derive(Deserialize)]
pub struct ClaimBaseHtlcRequest {
    pub swap_id: Option<String>,
    pub hash160: Option<String>,
    pub secret: Option<String>,
}

/// Validates whether an incoming HTTP request provides the correct session token
/// via header `X-Session-Token`, `Authorization: Bearer <token>`, or query param `token`.
pub fn verify_token_from_request(
    headers: &header::HeaderMap,
    uri: &axum::http::Uri,
    expected_token: &str,
) -> bool {
    if expected_token.is_empty() {
        return false;
    }

    // 1. Check X-Session-Token header
    if let Some(tok) = headers.get("x-session-token").and_then(|v| v.to_str().ok()) {
        if tok == expected_token {
            return true;
        }
    }

    // 2. Check Authorization: Bearer <token>
    if let Some(auth_hdr) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(tok) = auth_hdr.strip_prefix("Bearer ").or_else(|| auth_hdr.strip_prefix("bearer ")) {
            if tok.trim() == expected_token {
                return true;
            }
        }
    }

    // 3. Check query param: ?token=...
    if let Some(query) = uri.query() {
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == "token" && v == expected_token {
                    return true;
                }
            }
        }
    }

    false
}

/// Authentication middleware: enforces that mutating and sensitive API calls
/// possess the valid session token.
async fn require_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    if verify_token_from_request(req.headers(), req.uri(), &state.session_token) {
        return Ok(next.run(req).await);
    }

    Err((
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "success": false,
            "status": "error",
            "error": "Unauthorized: valid session token required in X-Session-Token, Authorization header, or token query param"
        })),
    ))
}

pub fn create_router(state: AppState) -> Router {
    let protected_routes = Router::new()
        .route("/api/vault/withdraw", post(post_vault_withdraw))
        .route("/api/vault/withdraw-base", post(super::handlers::post_vault_withdraw_base))
        .route("/api/vault/export", get(get_vault_export))
        .route("/api/swap/secret", get(super::handlers::get_swap_secret))
        .route("/api/swap/lock-htlc", post(super::handlers::post_lock_htlc))
        .route("/api/swap/claim-htlc", post(super::handlers::post_claim_htlc))
        .route("/api/swap/claim-vault-htlc", post(super::handlers::post_claim_vault_htlc))
        .route("/api/swap/lock-base-htlc", post(super::handlers::post_lock_base_htlc))
        .route("/api/swap/claim-base-htlc", post(super::handlers::post_claim_base_htlc))
        .route("/api/swap/mark_claimed", post(super::handlers::mark_swap_claimed))
        .route("/api/orderbook/create", post(super::handlers::post_create_order))
        .route("/api/orderbook/import", post(super::handlers::post_import_order))
        .route("/api/orderbook/cancel", post(super::handlers::post_cancel_order))
        .route("/api/orderbook/match", post(super::handlers::post_match_order))
        .route("/api/orderbook/take", post(super::handlers::post_take_order))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .route("/", get(serve_index))
        .route("/css/variables.css", get(|| serve_css(CSS_VARIABLES)))
        .route("/css/header.css", get(|| serve_css(CSS_HEADER)))
        .route("/css/cards.css", get(|| serve_css(CSS_CARDS)))
        .route("/css/modals.css", get(|| serve_css(CSS_MODALS)))
        .route("/js/config.js", get(|| serve_js(JS_CONFIG)))
        .route("/js/socket.js", get(|| serve_js(JS_SOCKET)))
        .route("/js/orderbook.js", get(|| serve_js(JS_ORDERBOOK)))
        .route("/js/swap.js", get(|| serve_js(JS_SWAP)))
        .route("/js/vault.js", get(|| serve_js(JS_VAULT)))
        .route("/js/qrcode.min.js", get(|| serve_js(JS_QRCODE)))
        .route("/js/jsqr.min.js", get(|| serve_js(JS_JSQR)))
        .route("/js/app.js", get(|| serve_js(JS_APP)))
        .route("/ws", get(ws_handler))
        .route("/api/orderbook", get(super::handlers::get_orderbook))
        .route("/api/swaps/ledger", get(get_swaps_ledger))
        .route("/api/mempool", get(get_mempool))
        .route("/api/vault/status", get(get_vault_status))
        .route("/api/vault/estimate-fee", get(get_estimate_fee))
        .route("/api/vault/base-gas-estimate", get(super::handlers::get_vault_base_gas_estimate))
        .route("/api/automation/logs", get(super::handlers::get_automation_logs))
        .route("/api/tx/:txid", get(super::handlers::get_tx_details))
        .route("/explorer", get(serve_explorer))
        .route("/js/explorer.js", get(|| serve_js(JS_EXPLORER)))
        .route("/api/blocks", get(super::handlers::get_blocks))
        .route("/api/block/:query", get(super::handlers::get_block_details))
        .route("/api/explorer/stats", get(super::handlers::get_explorer_stats))
        .route("/api/explorer/search", get(super::handlers::get_explorer_search))
        .route("/api/open_browser", get(handle_open_browser_get).post(handle_open_browser_post))
        .route("/docs", get(serve_docs))
        .merge(protected_routes)
        .with_state(state)
}

async fn serve_css(content: &'static str) -> impl IntoResponse {
    (
        [
            (axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (axum::http::header::CACHE_CONTROL, "no-cache, no-store, must-revalidate, max-age=0"),
        ],
        content,
    )
}

async fn serve_js(content: &'static str) -> impl IntoResponse {
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/javascript; charset=utf-8"),
            (axum::http::header::CACHE_CONTROL, "no-cache, no-store, must-revalidate, max-age=0"),
        ],
        content,
    )
}

async fn serve_index(
    State(state): State<AppState>,
    Query(_query): Query<AuthQuery>,
) -> impl IntoResponse {
    let html = HTML_CONTENT.replace(
        "let sessionToken = \"\";",
        &format!(
            "let sessionToken = \"{}\";\n    window.SERVER_SESSION_TOKEN = \"{}\";",
            state.session_token, state.session_token
        ),
    );
    (
        [
            (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (axum::http::header::CACHE_CONTROL, "no-cache, no-store, must-revalidate, max-age=0"),
            (axum::http::header::PRAGMA, "no-cache"),
            (axum::http::header::EXPIRES, "0"),
        ],
        html,
    )
}

async fn serve_explorer(
    State(state): State<AppState>,
    Query(_query): Query<AuthQuery>,
) -> impl IntoResponse {
    let html = EXPLORER_HTML.replace(
        "let sessionToken = \"\";",
        &format!(
            "let sessionToken = \"{}\";\n    window.SERVER_SESSION_TOKEN = \"{}\";",
            state.session_token, state.session_token
        ),
    );
    (
        [
            (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (axum::http::header::CACHE_CONTROL, "no-cache, no-store, must-revalidate, max-age=0"),
            (axum::http::header::PRAGMA, "no-cache"),
            (axum::http::header::EXPIRES, "0"),
        ],
        html,
    )
}

async fn serve_docs() -> impl IntoResponse {
    (
        [
            (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (axum::http::header::CACHE_CONTROL, "no-cache, no-store, must-revalidate, max-age=0"),
            (axum::http::header::PRAGMA, "no-cache"),
            (axum::http::header::EXPIRES, "0"),
        ],
        DOCS_HTML,
    )
}

async fn handle_open_browser_get(Query(query): Query<OpenBrowserQuery>) -> impl IntoResponse {
    let url = query.url.unwrap_or_default();
    let success = handle_open_url_request(&url);
    Json(serde_json::json!({
        "success": success,
        "url": url,
    }))
}

async fn handle_open_browser_post(Json(payload): Json<OpenBrowserPayload>) -> impl IntoResponse {
    let url = payload.url.unwrap_or_default();
    let success = handle_open_url_request(&url);
    Json(serde_json::json!({
        "success": success,
        "url": url,
    }))
}

fn handle_open_url_request(url: &str) -> bool {
    let trimmed = url.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return false;
    }
    // Prevent loopback self-triggers
    if trimmed.contains("127.0.0.1") || trimmed.contains("localhost") {
        return false;
    }
    crate::android::open_browser_url(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{header, HeaderMap, Uri};

    #[test]
    fn test_verify_token_from_request_all_modes() {
        let expected = "secret_auth_token_42";

        // 1. Missing credentials -> false
        let headers = HeaderMap::new();
        let uri: Uri = "/api/vault/withdraw".parse().unwrap();
        assert!(!verify_token_from_request(&headers, &uri, expected));

        // 2. Empty expected token -> always false (security safeguard)
        let mut h = HeaderMap::new();
        h.insert("x-session-token", "".parse().unwrap());
        assert!(!verify_token_from_request(&h, &uri, ""));

        // 3. Incorrect X-Session-Token -> false
        let mut h = HeaderMap::new();
        h.insert("x-session-token", "evil_hacker_token".parse().unwrap());
        assert!(!verify_token_from_request(&h, &uri, expected));

        // 4. Valid X-Session-Token -> true
        let mut h = HeaderMap::new();
        h.insert("x-session-token", expected.parse().unwrap());
        assert!(verify_token_from_request(&h, &uri, expected));

        // 5. Valid Authorization: Bearer <token> -> true
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", expected).parse().unwrap(),
        );
        assert!(verify_token_from_request(&h, &uri, expected));

        // 6. Valid Authorization: bearer (case-insensitive prefix) -> true
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            format!("bearer {}", expected).parse().unwrap(),
        );
        assert!(verify_token_from_request(&h, &uri, expected));

        // 7. Invalid Authorization token -> false
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            "Bearer invalid_secret".parse().unwrap(),
        );
        assert!(!verify_token_from_request(&h, &uri, expected));

        // 8. Valid query parameter ?token=... -> true
        let h = HeaderMap::new();
        let uri_with_token: Uri = format!("/api/vault/withdraw?token={}", expected)
            .parse()
            .unwrap();
        assert!(verify_token_from_request(&h, &uri_with_token, expected));

        // 9. Query param with multiple params -> true
        let uri_multi: Uri = format!("/api/orderbook/cancel?id=123&token={}&force=true", expected)
            .parse()
            .unwrap();
        assert!(verify_token_from_request(&h, &uri_multi, expected));

        // 10. Query param with wrong token -> false
        let uri_wrong: Uri = "/api/vault/withdraw?token=fake_token".parse().unwrap();
        assert!(!verify_token_from_request(&h, &uri_wrong, expected));
    }
}
