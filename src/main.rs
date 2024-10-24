use async_nats::{Client, ConnectOptions};
use chrono::Utc;
use futures::stream::FuturesUnordered;
use futures_util::{sink::SinkExt, stream::StreamExt};
use log::LevelFilter;
use log::{debug, error, info};
use rusqlite::Connection;
use serde::Serialize;
use simple_logger::SimpleLogger;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};
use warp::filters::ws::WebSocket;
use warp::reject::Reject;
use warp::ws::Message;
use warp::Filter;

pub mod datatypes;
mod sql;
use datatypes::*;

#[derive(Debug, Clone)]
struct ServerError<E: 'static + std::error::Error + Sync + Send + Debug> {
    #[allow(dead_code)]
    error: E,
}

impl<E: 'static + std::error::Error + Sync + Send + Debug> Reject for ServerError<E> {}

impl<E: 'static + std::error::Error + Sync + Send + Debug> From<E> for ServerError<E> {
    fn from(error: E) -> Self {
        ServerError { error }
    }
}

async fn connect_to_nats(
    url: &str,
    token: Option<String>,
) -> Result<Client, Box<dyn std::error::Error>> {
    debug!("Attempting to connect to NATS server at {}", url);
    let mut options = ConnectOptions::new();

    if let Some(auth_token) = token {
        debug!("Using authentication token for NATS connection");
        options = options.token(auth_token);
    } else {
        debug!("No authentication token provided for NATS connection");
    }

    match options.connect(url).await {
        Ok(client) => {
            info!("Successfully connected to NATS server at {}", url);
            Ok(client)
        }
        Err(e) => {
            error!("Failed to connect to NATS server at {}: {:?}", url, e);
            Err(e.into())
        }
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> rusqlite::Result<()> {
    // Set this to false to disable debug logging
    let enable_debug = true;

    // Setup the logger with UTC timestamps
    SimpleLogger::new()
        .with_level(if enable_debug {
            LevelFilter::Debug
        } else {
            LevelFilter::Info
        })
        .with_utc_timestamps()
        .init()
        .unwrap();

    if enable_debug {
        info!("Logger initialized at Debug level");
    } else {
        info!("Logger initialized at Info level");
    }

    // Setup the database
    let db_conn = sql::get_db_conn()?;
    sql::db_setup(&db_conn)?;
    let db_conn_filter = warp::any().map(|| sql::db_conn());

    // Setup global app state for sharing between threads
    let state = Arc::new(Mutex::new(App::default()));
    debug!("Global app state initialized");
    let state_clone = Arc::clone(&state);
    let state_filter = warp::any().map(move || Arc::clone(&state_clone));

    let builder = reqwest::ClientBuilder::new().connect_timeout(Duration::new(0, 250_000_000));
    let client = builder.build().expect("Failed to build reqwest client");

    // Setup a concurrent running thread that calls monitoring endpoints
    // on configured NATS servers every second.
    let (tx, _) = broadcast::channel::<VarzBroadcastMessage>(16);
    let sender_clone = tx.clone();
    let receiver_filter = warp::any().map(move || tx.subscribe());

    let state_for_spawn = Arc::clone(&state);
    tokio::spawn(async move {
        debug!("Starting server monitoring thread");
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        let s = state_for_spawn;
        let tx = sender_clone;
        let cl = client.clone();
        loop {
            interval.tick().await;
            let state = Arc::clone(&s);
            let sender = tx.clone();
            let client = cl.clone();
            tokio::spawn(async move {
                debug!("Fetching server varz");
                if let Err(e) = get_server_varz(state, sender, client).await {
                    error!("Error fetching server varz: {:?}", e);
                }
            });
        }
    });

    // GET /<anything>
    let static_content_route = warp::any().and(warp::get()).and(warp::fs::dir("web/dist"));

    // GET /api/state
    let get_app_state_route = warp::path::end()
        .and(warp::get())
        .and(db_conn_filter.clone())
        .and(state_filter.clone())
        .and_then(get_state);

    // POST /api/state/client/new
    let new_client_route = warp::path!("client" / "new")
        .and(warp::post())
        .and(db_conn_filter.clone())
        .and(warp::body::json::<NatsClient>())
        .and_then(handle_insert_client);

    // POST /api/state/client/update
    let update_client_route = warp::path!("client" / "update")
        .and(warp::post())
        .and(db_conn_filter.clone())
        .and(warp::body::json::<NatsClient>())
        .and_then(handle_update_client);

    // GET /api/state/client/delete/<id>
    let delete_client_route = warp::path!("client" / "delete" / i64)
        .and(warp::get())
        .and(db_conn_filter.clone())
        .and_then(handle_delete_client);

    // POST /api/state/server/new
    let new_server_route = warp::path!("server" / "new")
        .and(warp::post())
        .and(db_conn_filter.clone())
        .and(warp::body::json::<NatsServer>())
        .and_then(handle_insert_server);

    // POST /api/state/server/update
    let update_server_route = warp::path!("server" / "update")
        .and(warp::post())
        .and(db_conn_filter.clone())
        .and(warp::body::json::<NatsServer>())
        .and_then(handle_update_server);

    // GET /api/state/server/delete/<id>
    let delete_server_route = warp::path!("server" / "delete" / i64)
        .and(warp::get())
        .and(db_conn_filter.clone())
        .and_then(handle_delete_server);

    // GET /client with websocket upgrade
    let client_subscribe_route = warp::path!("client" / i64)
        .and(warp::ws())
        .and(db_conn_filter.clone())
        .and_then(handle_client_subscribe_request);

    // GET /api/state/ws websocket
    let transient_info_route = warp::path!("ws")
        .and(warp::path::end())
        .and(warp::ws())
        .and(receiver_filter)
        .map(
            |ws: warp::ws::Ws, rx: broadcast::Receiver<VarzBroadcastMessage>| {
                ws.on_upgrade(|ws: WebSocket| async move { broadcast_transient_info(ws, rx).await })
            },
        );

    let api_route = warp::path("api").and(warp::path("state")).and(
        get_app_state_route
            .or(new_client_route)
            .or(update_client_route)
            .or(delete_client_route)
            .or(new_server_route)
            .or(update_server_route)
            .or(delete_server_route)
            .or(transient_info_route),
    );

    let route = static_content_route
        .or(api_route)
        .or(client_subscribe_route)
        .with(warp::log("web"));

    debug!("Starting server on 0.0.0.0:80");
    warp::serve(route).run(([0, 0, 0, 0], 80)).await;

    Ok(())
}

async fn get_state(
    conn: Connection,
    state: Arc<Mutex<App>>,
) -> Result<impl warp::Reply, warp::Rejection> {
    debug!("Fetching application state");
    let (svs, cls) = tokio::task::spawn_blocking(move || {
        let svs = sql::get_servers(&conn).map_err(ServerError::from)?;
        let cls = sql::get_clients(&conn).map_err(ServerError::from)?;
        Ok::<_, ServerError<rusqlite::Error>>((svs, cls))
    })
    .await
    .map_err(|e| warp::reject::custom(ServerError::from(e)))?
    .map_err(warp::reject::custom)?;

    let mut st = state.lock().await;
    st.set_servers(svs);
    st.set_clients(cls);
    Ok(warp::reply::json(&st.clone()))
}

async fn broadcast_transient_info(
    mut ws: WebSocket,
    mut rx: broadcast::Receiver<VarzBroadcastMessage>,
) {
    debug!("Starting transient info broadcast");
    while let Ok(msg) = rx.recv().await {
        debug!("Received varz update for server ID: {}", msg.server_id);
        match serde_json::to_string(&msg) {
            Ok(json) => {
                debug!("Serialized varz update: {}", json);
                if let Err(e) = ws.send(Message::text(json)).await {
                    error!("Error sending WebSocket message: {:?}", e);
                    break;
                }
            }
            Err(e) => error!("Error serializing varz update: {:?}", e),
        }
    }
    debug!("Transient info broadcast ended");
}

async fn get_server_varz(
    state: Arc<Mutex<App>>,
    tx: broadcast::Sender<VarzBroadcastMessage>,
    client: reqwest::Client,
) -> Result<(), Box<dyn std::error::Error>> {
    debug!("Fetching server varz");
    let servers = state.lock().await.servers.clone();
    debug!("Number of servers to fetch varz: {}", servers.len());
    let mut stream = servers
        .iter()
        .map(|s| {
            debug!("Fetching varz for server ID: {}", s.id.unwrap());
            NatsServer::get_varz(s.id.unwrap(), s.host.clone(), s.monitoring_port, &client)
        })
        .collect::<FuturesUnordered<_>>();
    while let Some(result) = stream.next().await {
        match result {
            Ok(v) => {
                debug!("Received varz update for server ID: {}", v.server_id);
                if let Err(e) = tx.send(v) {
                    error!("Failed to send app state message: {:?}", e);
                }
            }
            Err(e) => error!("Failed to fetch varz: {:?}", e),
        }
    }
    debug!("Finished fetching server varz");
    Ok(())
}

async fn handle_insert_client(
    conn: Connection,
    client: NatsClient,
) -> Result<impl warp::Reply, warp::Rejection> {
    debug!("Inserting new client: {:?}", client);
    sql::insert_client(&conn, client).map_err(|e| warp::reject::custom(ServerError::from(e)))?;
    Ok(warp::reply())
}

async fn handle_update_client(
    conn: Connection,
    client: NatsClient,
) -> Result<impl warp::Reply, warp::Rejection> {
    debug!("Updating client: {:?}", client);
    match sql::update_client(&conn, client) {
        Ok(_) => Ok(warp::reply()),
        Err(e) => Err(ServerError::from(e).into()),
    }
}

async fn handle_delete_client(
    client_id: i64,
    conn: Connection,
) -> Result<impl warp::Reply, warp::Rejection> {
    debug!("Deleting client with ID: {}", client_id);
    match sql::delete_client(&conn, client_id) {
        Ok(_) => Ok(warp::reply()),
        Err(e) => Err(ServerError::from(e).into()),
    }
}

async fn handle_insert_server(
    conn: Connection,
    server: NatsServer,
) -> Result<impl warp::Reply, warp::Rejection> {
    match sql::insert_server(&conn, server) {
        Ok(_) => Ok(warp::reply()),
        Err(e) => Err(ServerError::from(e).into()),
    }
}

async fn handle_update_server(
    conn: Connection,
    server: NatsServer,
) -> Result<impl warp::Reply, warp::Rejection> {
    match sql::update_server(&conn, server) {
        Ok(_) => Ok(warp::reply()),
        Err(e) => Err(ServerError::from(e).into()),
    }
}

async fn handle_delete_server(
    server_id: i64,
    conn: Connection,
) -> Result<impl warp::Reply, warp::Rejection> {
    match sql::delete_server(&conn, server_id) {
        Ok(_) => Ok(warp::reply()),
        Err(e) => Err(ServerError::from(e).into()),
    }
}

#[derive(Debug, Clone, Serialize)]
struct SocketMessage {
    typ: SocketMessageType,
    timestamp: i64,
    subject: Option<String>,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
enum SocketMessageType {
    Msg,
    Info,
    Ping,
    Pong,
    Ok,
    Err,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
struct SubscriptionMessage {
    payload: String,
    subject: String,
}

async fn handle_client_subscribe_request(
    client_id: i64,
    ws: warp::ws::Ws,
    conn: Connection,
) -> Result<impl warp::Reply, warp::Rejection> {
    debug!("Handling subscription request for client ID: {}", client_id);
    match sql::get_connection_triple(&conn, client_id) {
        Ok((hostname, port, subjects, token)) => {
            let addr = format!("{}:{}", hostname, port).parse().unwrap();
            Ok(ws.on_upgrade(|ws| async move {
                let sbjs = subjects
                    .into_iter()
                    .flat_map(|node| node.flatten())
                    .collect::<Vec<String>>();
                handle_client_subscription(ws, addr, sbjs, token).await
            }))
        }
        Err(e) => Err(ServerError::from(e).into()),
    }
}

async fn handle_client_subscription(
    mut ws: WebSocket,
    dest: String,
    subjects: Vec<String>,
    token: Option<String>,
) {
    debug!(
        "Starting client subscription to {} for subjects: {:?}",
        dest, subjects
    );
    let client = match connect_to_nats(&dest, token).await {
        Ok(c) => {
            info!("Successfully connected to NATS server for subscription");
            c
        }
        Err(e) => {
            error!("Failed to connect to NATS server: {:?}", e);
            return;
        }
    };

    let mut subscriptions = Vec::new();
    for subject in subjects {
        match client.subscribe(subject.clone()).await {
            Ok(sub) => {
                debug!("Successfully subscribed to subject: {}", subject);
                subscriptions.push(sub);
            }
            Err(e) => {
                error!("Failed to subscribe to subject {}: {:?}", subject, e);
                continue;
            }
        }
    }

    let mut stream = futures::stream::select_all(subscriptions);

    while let Some(msg) = stream.next().await {
        debug!("Received message on subject: {}", msg.subject);
        let socket_message = SocketMessage {
            typ: SocketMessageType::Msg,
            timestamp: Utc::now().timestamp_millis(),
            subject: Some(msg.subject.to_string()),
            message: String::from_utf8_lossy(&msg.payload).to_string(),
        };

        if let Err(e) = ws
            .send(Message::text(
                serde_json::to_string(&socket_message).unwrap(),
            ))
            .await
        {
            error!("WebSocket send error: {:?}", e);
            break;
        }
    }

    info!("Subscription to {} has ended.", dest);
}
