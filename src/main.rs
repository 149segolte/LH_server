use axum::body::Body;
use axum::extract::State;
use axum::{
    extract::Json,
    http::{header::CONTENT_TYPE, HeaderMap, Method, Request, StatusCode},
    routing::{get, post},
    Router,
};
use mysql::prelude::Queryable;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

struct AppState {
    db_pool: mysql::Pool,
}

#[tokio::main]
async fn main() {
    // initialize tracing
    let file_appender = tracing_appender::rolling::hourly("./logs", "app.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(non_blocking)
        .init();

    // initialize database connection
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL not found");
    let builder = mysql::OptsBuilder::from_opts(mysql::Opts::from_url(&url).unwrap());
    let pool = mysql::Pool::new(builder.ssl_opts(mysql::SslOpts::default())).unwrap();
    let _conn = pool.get_conn().unwrap();
    println!("Successfully connected to PlanetScale!");

    // create our application state
    let app_state = Arc::new(Mutex::new(AppState { db_pool: pool }));

    // build our application with a router
    let app = Router::new()
        .route("/log", get(list))
        .route("/log/new", post(logs))
        .route("/log/new2", post(logs2))
        .layer(
            CorsLayer::new()
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([CONTENT_TYPE])
                .allow_origin(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);

    // run it with hyper on localhost
    let addr = ([127, 0, 0, 1], port).into();
    let server = axum::Server::bind(&addr);
    tracing::debug!("Listening on {}", addr);
    server.serve(app.into_make_service()).await.unwrap();
}

#[derive(serde::Deserialize, Debug)]
struct Log {
    lang: String,
    level: String,
    message: String,
    button: u8,
    retention: u8,
}

async fn list(State(state): State<Arc<Mutex<AppState>>>) -> (StatusCode, Json<Value>) {
    let app_state = state.lock().await;
    let mut conn = app_state.db_pool.get_conn().unwrap();
    let logs: Vec<(String, String, String, String, String, String, String)> = conn
        .query_map(
            "SELECT * FROM logdetails ORDER BY date DESC, time DESC LIMIT 4",
            |(date, time, level, host_adr, header, message, reten_date)| {
                (date, time, level, host_adr, header, message, reten_date)
            },
        )
        .unwrap();
    (StatusCode::OK, Json(json!(logs)))
}

async fn logs(
    State(state): State<Arc<Mutex<AppState>>>,
    headers: HeaderMap,
    log: Json<Log>,
) -> (StatusCode, Json<Value>) {
    let date = chrono::Local::now().date_naive();
    let time = chrono::Local::now().time();
    let headers: Vec<String> = headers
        .iter()
        .map(|(key, value)| format!("\"{}\": \"{}\"", key, value.to_str().unwrap()))
        .collect();
    let mut header = "{".to_owned() + &headers.join(", ") + "}";
    let headers = serde_json::from_str::<serde_json::Value>(&header).unwrap();
    tracing::info!("Received headers: {:?}", header);
    header.truncate(98);
    tracing::event!(
        tracing::Level::INFO,
        lang = log.lang.as_str(),
        "Received log: {:?}",
        log
    );
    let app_state = state.lock().await;
    let query = format!(
        "INSERT INTO logdetails (date, time, level, host_adr, header, message, reten_date) VALUES ('{}', '{}', '{}', {}, '{}', '{}', '{}')",
        date, time, log.level, headers["x-forwarded-for"], header, log.message, date + chrono::Duration::days(log.retention.into())
    );
    let mut conn = app_state.db_pool.get_conn().unwrap();
    conn.query_drop(query).unwrap();
    (StatusCode::CREATED, Json(json!({})))
}

async fn logs2(req: Request<Body>) -> (StatusCode, Json<Value>) {
    tracing::info!("Received: {:?}", req);
    tracing::info!("Received headers: {:?}", req.headers());
    tracing::info!("Received log: {:?}", req.body());
    (StatusCode::CREATED, Json(json!({})))
}
