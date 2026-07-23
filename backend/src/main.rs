mod content;
mod engine;
mod model;

use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::broadcast;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing::{error, info, warn};

use crate::{
    content::Catalog,
    engine::Engine,
    model::{GameState, SubmittedAnswer},
};

#[derive(Parser, Debug)]
#[command(version, about = "Hoot! local-network quiz server")]
struct Args {
    #[arg(long, env = "HOOT_GAMES", default_value = "content/games.json")]
    games: PathBuf,
    #[arg(long, env = "HOOT_BIND", default_value = "0.0.0.0:8080")]
    bind: SocketAddr,
    #[arg(long, env = "HOOT_FRONTEND", default_value = "frontend/dist")]
    frontend: PathBuf,
    #[arg(long, env = "HOOT_PUBLIC_URL")]
    public_url: Option<String>,
}

#[derive(Clone)]
struct AppState {
    engine: Arc<Mutex<Engine>>,
    recovery_key: Arc<String>,
    updates: broadcast::Sender<u64>,
}

#[derive(Debug)]
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JoinRequest {
    username: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimRequest {
    recovery_key: Option<String>,
}

#[derive(Deserialize)]
struct AuthMessage {
    #[serde(rename = "type")]
    message_type: String,
    role: String,
    token: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientCommand {
    SubmitMultipleChoice { option_id: String },
    SubmitFreeText { text: String },
    HostSelectGame { game_id: String },
    HostStart,
    HostAdvance,
    HostReloadContent,
    HostSetJoinUrl { url: String },
    Ping { client_time_ms: Option<i64> },
}

#[derive(Clone)]
enum ClientIdentity {
    Host(String),
    Player(String),
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hoot_server=info,tower_http=info".into()),
        )
        .init();
    let args = Args::parse();
    let catalog = Catalog::load(&args.games)?;
    let recovery_key = recovery_key();
    let join_urls = detect_join_urls(args.bind, args.public_url.as_deref());
    let network_warning = wsl2_network_warning(args.public_url.as_deref());
    let state = GameState {
        advertised_url: join_urls.first().cloned(),
        network_warning: network_warning.clone(),
        ..GameState::default()
    };
    let engine = Engine::new(state, catalog.clone(), join_urls.clone());

    let (updates, _) = broadcast::channel(256);
    let app_state = AppState {
        engine: Arc::new(Mutex::new(engine)),
        recovery_key: Arc::new(recovery_key.clone()),
        updates,
    };
    spawn_clock(app_state.clone());

    let index = args.frontend.join("index.html");
    let static_files = ServeDir::new(&args.frontend).fallback(ServeFile::new(index));
    let media_files = ServeDir::new(&catalog.media_root);
    let app = Router::new()
        .route(
            "/api/health",
            get(|| async { Json(json!({ "status": "ok" })) }),
        )
        .route("/api/players/join", post(join_player))
        .route("/api/host/claim", post(claim_host))
        .route("/api/ws", get(websocket))
        .nest_service("/media", media_files)
        .fallback_service(static_files)
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    info!("Host recovery key: {recovery_key}");
    for url in &join_urls {
        info!("Join URL: {url}");
    }
    if let Some(warning) = &network_warning {
        warn!("{warning}");
    }
    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("could not bind {}", args.bind))?;
    info!("Hoot server listening on {}", args.bind);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn join_player(
    State(app): State<AppState>,
    Json(request): Json<JoinRequest>,
) -> Result<Json<Value>, ApiError> {
    let token = random_token();
    let mut engine = app.engine.lock().map_err(lock_error)?;
    let player_id = engine
        .join_player(&request.username, hash_token(&token))
        .map_err(|message| ApiError(StatusCode::CONFLICT, message))?;
    let revision = engine.state.revision;
    drop(engine);
    let _ = app.updates.send(revision);
    Ok(Json(json!({ "token": token, "playerId": player_id })))
}

async fn claim_host(
    State(app): State<AppState>,
    Json(request): Json<ClaimRequest>,
) -> Result<Json<Value>, ApiError> {
    let token = random_token();
    let mut engine = app.engine.lock().map_err(lock_error)?;
    if engine.state.host_token_hash.is_some() {
        let supplied = request.recovery_key.unwrap_or_default();
        if !constant_eq(supplied.trim(), app.recovery_key.as_str()) {
            return Err(ApiError(
                StatusCode::UNAUTHORIZED,
                "The recovery key is not valid.".into(),
            ));
        }
    }
    engine.state.host_token_hash = Some(hash_token(&token));
    engine.bump();
    let revision = engine.state.revision;
    drop(engine);
    let _ = app.updates.send(revision);
    Ok(Json(json!({ "token": token })))
}

async fn websocket(ws: WebSocketUpgrade, State(app): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, app))
}

async fn handle_socket(mut socket: WebSocket, app: AppState) {
    let auth = tokio::time::timeout(Duration::from_secs(8), socket.recv()).await;
    let identity = match auth {
        Ok(Some(Ok(Message::Text(raw)))) => match serde_json::from_str::<AuthMessage>(&raw) {
            Ok(message) if message.message_type == "authenticate" => authenticate(&app, &message),
            _ => None,
        },
        _ => None,
    };
    let Some(identity) = identity else {
        let _ = send_json(&mut socket, json!({ "type": "error", "code": "authentication_failed", "message": "Session token was not accepted." })).await;
        let _ = socket.close().await;
        return;
    };

    connection_opened(&app, &identity);
    let mut receiver = app.updates.subscribe();
    let (mut sender, mut incoming) = socket.split();
    if let Err(error) = send_snapshot(&mut sender, &app, &identity).await {
        warn!("could not send initial snapshot: {error}");
        connection_closed(&app, &identity);
        return;
    }

    loop {
        tokio::select! {
            message = incoming.next() => {
                match message {
                    Some(Ok(Message::Text(raw))) => {
                        match serde_json::from_str::<ClientCommand>(&raw) {
                            Ok(command) => {
                                if let Err(message) = execute_command(&app, &identity, command) {
                                    let _ = sender.send(Message::Text(json!({ "type": "error", "message": message }).to_string().into())).await;
                                }
                            }
                            Err(_) => {
                                let _ = sender.send(Message::Text(json!({ "type": "error", "message": "That command was not understood." }).to_string().into())).await;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
            update = receiver.recv() => {
                match update {
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        if send_snapshot(&mut sender, &app, &identity).await.is_err() { break; }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    connection_closed(&app, &identity);
}

fn authenticate(app: &AppState, message: &AuthMessage) -> Option<ClientIdentity> {
    let token_hash = hash_token(&message.token);
    let engine = app.engine.lock().ok()?;
    match message.role.as_str() {
        "host" if engine.state.host_token_hash.as_deref() == Some(token_hash.as_str()) => {
            Some(ClientIdentity::Host(token_hash))
        }
        "player" => engine
            .player_by_token_hash(&token_hash)
            .map(|player| ClientIdentity::Player(player.id.clone())),
        _ => None,
    }
}

fn execute_command(
    app: &AppState,
    identity: &ClientIdentity,
    command: ClientCommand,
) -> Result<(), String> {
    if let ClientCommand::Ping { client_time_ms } = command {
        let _ = client_time_ms;
        let _ = app.updates.send(
            app.engine
                .lock()
                .map_err(|_| "Server state is unavailable.")?
                .state
                .revision,
        );
        return Ok(());
    }
    let mut engine = app
        .engine
        .lock()
        .map_err(|_| "Server state is unavailable.")?;
    if let ClientIdentity::Host(token_hash) = identity
        && engine.state.host_token_hash.as_deref() != Some(token_hash.as_str())
    {
        return Err("Host control has been transferred to another browser.".into());
    }
    match (identity, command) {
        (ClientIdentity::Player(id), ClientCommand::SubmitMultipleChoice { option_id }) => {
            engine.submit(id, SubmittedAnswer::MultipleChoice { option_id }, now_ms())?;
        }
        (ClientIdentity::Player(id), ClientCommand::SubmitFreeText { text }) => {
            engine.submit(id, SubmittedAnswer::FreeText { text }, now_ms())?;
        }
        (ClientIdentity::Host(_), ClientCommand::HostSelectGame { game_id }) => {
            engine.select_game(&game_id)?
        }
        (ClientIdentity::Host(_), ClientCommand::HostStart) => engine.start_game(now_ms())?,
        (ClientIdentity::Host(_), ClientCommand::HostAdvance) => engine.advance_host(now_ms())?,
        (ClientIdentity::Host(_), ClientCommand::HostReloadContent) => engine.reload_catalog()?,
        (ClientIdentity::Host(_), ClientCommand::HostSetJoinUrl { url }) => {
            engine.set_advertised_url(url)?
        }
        _ => return Err("This session is not allowed to use that command.".into()),
    }
    let revision = engine.state.revision;
    drop(engine);
    let _ = app.updates.send(revision);
    Ok(())
}

fn connection_opened(app: &AppState, identity: &ClientIdentity) {
    if let Ok(mut engine) = app.engine.lock() {
        match identity {
            ClientIdentity::Host(_) => engine.host_connections += 1,
            ClientIdentity::Player(id) => {
                *engine.connected_players.entry(id.clone()).or_default() += 1
            }
        }
        let _ = app.updates.send(engine.state.revision);
    }
}

fn connection_closed(app: &AppState, identity: &ClientIdentity) {
    if let Ok(mut engine) = app.engine.lock() {
        match identity {
            ClientIdentity::Host(_) => {
                engine.host_connections = engine.host_connections.saturating_sub(1)
            }
            ClientIdentity::Player(id) => {
                if let Some(count) = engine.connected_players.get_mut(id) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        engine.connected_players.remove(id);
                    }
                }
            }
        }
        let _ = app.updates.send(engine.state.revision);
    }
}

async fn send_snapshot<S>(sender: &mut S, app: &AppState, identity: &ClientIdentity) -> Result<()>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let state = {
        let engine = app
            .engine
            .lock()
            .map_err(|_| anyhow::anyhow!("state lock poisoned"))?;
        match identity {
            ClientIdentity::Host(token_hash)
                if engine.state.host_token_hash.as_deref() == Some(token_hash.as_str()) =>
            {
                engine.host_snapshot(now_ms())
            }
            ClientIdentity::Host(_) => anyhow::bail!("host control was transferred"),
            ClientIdentity::Player(id) => engine
                .player_snapshot(id, now_ms())
                .map_err(anyhow::Error::msg)?,
        }
    };
    sender
        .send(Message::Text(
            json!({ "type": "snapshot", "state": state })
                .to_string()
                .into(),
        ))
        .await?;
    Ok(())
}

async fn send_json(socket: &mut WebSocket, value: Value) -> Result<()> {
    socket.send(Message::Text(value.to_string().into())).await?;
    Ok(())
}

fn spawn_clock(app: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        loop {
            interval.tick().await;
            let mut engine = match app.engine.lock() {
                Ok(engine) => engine,
                Err(_) => {
                    error!("clock transition failed: state lock poisoned");
                    continue;
                }
            };
            match engine.tick(now_ms()) {
                Ok(true) => {
                    let _ = app.updates.send(engine.state.revision);
                }
                Ok(false) => {}
                Err(error) => error!("clock transition failed: {error}"),
            }
        }
    });
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> ApiError {
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Server state is unavailable.".into(),
    )
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn recovery_key() -> String {
    let mut bytes = [0_u8; 12];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn constant_eq(left: &str, right: &str) -> bool {
    let left_hash = Sha256::digest(left.as_bytes());
    let right_hash = Sha256::digest(right.as_bytes());
    bool::from(left_hash.ct_eq(&right_hash))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn detect_join_urls(bind: SocketAddr, public_url: Option<&str>) -> Vec<String> {
    let mut urls = Vec::new();
    if let Some(url) = public_url {
        urls.push(url.trim_end_matches('/').to_owned());
    }
    let mut addresses = match bind.ip() {
        IpAddr::V4(address) if address.is_loopback() => vec![address],
        IpAddr::V4(address) if !address.is_unspecified() => vec![address],
        _ => local_ip_address::list_afinet_netifas()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(_, address)| match address {
                IpAddr::V4(address) if !address.is_loopback() => Some(address),
                _ => None,
            })
            .collect::<Vec<Ipv4Addr>>(),
    };
    addresses.sort_by_key(|address| (!address.is_private(), *address));
    let mut seen = HashSet::new();
    for address in addresses {
        let url = format!("http://{address}:{}", bind.port());
        if seen.insert(url.clone()) && !urls.contains(&url) {
            urls.push(url);
        }
    }
    if urls.is_empty() {
        urls.push(format!("http://127.0.0.1:{}", bind.port()));
    }
    urls
}

fn is_wsl2_version_string(contents: &str) -> bool {
    let lower = contents.to_lowercase();
    lower.contains("microsoft") || lower.contains("wsl")
}

fn is_wsl2() -> bool {
    std::fs::read_to_string("/proc/version")
        .map(|contents| is_wsl2_version_string(&contents))
        .unwrap_or(false)
}

fn wsl2_network_warning(public_url: Option<&str>) -> Option<String> {
    if public_url.is_some() || !is_wsl2() {
        return None;
    }
    Some(
        "This server is running inside WSL2. The auto-detected address(es) above are \
         WSL2-internal and are usually unreachable from phones on your Wi-Fi. In Windows \
         PowerShell, run `ipconfig` to find your real Wi-Fi/Ethernet adapter IPv4 address, then \
         either restart Hoot with HOOT_PUBLIC_URL=http://<that-ip>:8080, or enable WSL2 \
         'mirrored' networking mode (Windows 11 22H2+: add networkingMode=mirrored under [wsl2] \
         in .wslconfig). You may also need a Windows Firewall inbound rule for this port."
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_loopback_bind_is_not_advertised_as_lan_accessible() {
        let urls = detect_join_urls("127.0.0.1:8123".parse().unwrap(), None);
        assert_eq!(urls, vec!["http://127.0.0.1:8123"]);
    }

    #[test]
    fn configured_public_url_takes_precedence() {
        let urls = detect_join_urls(
            "127.0.0.1:8123".parse().unwrap(),
            Some("http://hoot.local:8123/"),
        );
        assert_eq!(urls[0], "http://hoot.local:8123");
    }

    #[test]
    fn wsl2_kernel_version_string_is_detected() {
        let version = "Linux version 6.6.87.2-microsoft-standard-WSL2 (root@439a258ad544) \
             (gcc (GCC) 11.2.0, GNU ld (GNU Binutils) 2.37) #1 SMP PREEMPT_DYNAMIC";
        assert!(is_wsl2_version_string(version));
    }

    #[test]
    fn generic_linux_kernel_version_string_is_not_detected() {
        let version =
            "Linux version 6.1.0-13-amd64 (debian-kernel@lists.debian.org) (gcc-12) #1 SMP";
        assert!(!is_wsl2_version_string(version));
    }
}
