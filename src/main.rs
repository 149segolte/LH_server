use axum::extract::State;
use axum::{
    extract::Json,
    http::{header::CONTENT_TYPE, HeaderMap, Method, StatusCode},
    routing::{get, post},
    Router,
};
use hyper::body::Bytes;
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
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_max_level(tracing::Level::DEBUG)
        .init();

    // initialize database connection
    let url = std::env::var("DATABASE_URL").expect(
        format!(
            "DATABASE_URL not set {:?}",
            std::env::vars().collect::<Vec<(String, String)>>()
        )
        .as_str(),
    );
    let builder = mysql::OptsBuilder::from_opts(mysql::Opts::from_url(&url).unwrap());
    let pool = mysql::Pool::new(builder.ssl_opts(mysql::SslOpts::default())).unwrap();
    let _conn = pool.get_conn().unwrap();
    tracing::info!("Connected to database");

    // create our application state
    let app_state = Arc::new(Mutex::new(AppState { db_pool: pool }));

    // build our application with a router
    let app = Router::new()
        .route("/", get(|| async { "Welcome!" }))
        .route("/log", get(list))
        .route("/log/new", post(logs))
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
        .unwrap_or(8080);

    // run it with hyper on localhost
    let addr = format!("[::]:{}", port).parse().unwrap();
    let server = axum::Server::bind(&addr);
    tracing::debug!("Listening on {}", addr);
    server.serve(app.into_make_service()).await.unwrap();

    tracing::info!("Shutting down");
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct Log {
    date: String,
    time: String,
    level: String,
    address: String,
    header: String,
    message: String,
    retention: String,
    button: u8,
    lang: String,
}

impl Default for Log {
    fn default() -> Self {
        Self {
            date: chrono::Local::now().date_naive().to_string(),
            time: chrono::Local::now().time().to_string(),
            level: "info".to_string(),
            address: "".to_string(),
            header: "".to_string(),
            message: "".to_string(),
            retention: (chrono::Local::now().date_naive() + chrono::Duration::days(7)).to_string(),
            button: 0,
            lang: "python".to_string(),
        }
    }
}

impl Log {
    fn new(address: String, header: String, request: Json<Value>) -> Self {
        let mut log = Self::default();
        log.level = request["level"].to_string();
        log.address = address;
        log.header = header;
        log.message = request["message"].to_string();
        log.retention = (chrono::NaiveDate::parse_from_str(log.date.as_str(), "%Y-%m-%d").unwrap()
            + chrono::Duration::days(request["retention"].as_u64().unwrap() as i64))
        .to_string();
        log.button = request["button"].as_u64().unwrap() as u8;
        log.lang = request["lang"].to_string();
        log
    }
}

async fn list(State(state): State<Arc<Mutex<AppState>>>) -> (StatusCode, Json<Value>) {
    let app_state = state.lock().await;
    let mut conn = app_state.db_pool.get_conn().unwrap();
    let logs: Vec<Log> = conn
        .query_map(
            "SELECT * FROM logdetails ORDER BY date DESC, time DESC LIMIT 4",
            |row| {
                let (date, time, level, host_adr, header, message, reten_date): (
                    Vec<u8>,
                    Vec<u8>,
                    Vec<u8>,
                    Vec<u8>,
                    Vec<u8>,
                    Vec<u8>,
                    Vec<u8>,
                ) = mysql::from_row(row);
                Log {
                    date: String::from_utf8(date).unwrap(),
                    time: String::from_utf8(time).unwrap(),
                    level: String::from_utf8(level).unwrap(),
                    address: String::from_utf8(host_adr).unwrap(),
                    header: String::from_utf8(header).unwrap(),
                    message: String::from_utf8(message).unwrap(),
                    retention: String::from_utf8(reten_date).unwrap(),
                    button: 0,
                    lang: "python".to_string(),
                }
            },
        )
        .unwrap();
    tracing::debug!("Fetching logs: {:?}", logs);
    (StatusCode::OK, Json(json!(logs)))
}

async fn logs(
    State(state): State<Arc<Mutex<AppState>>>,
    headers: HeaderMap,
    log: Json<Value>,
) -> (StatusCode, Json<Value>) {
    let headers: Vec<String> = headers
        .iter()
        .map(|(key, value)| format!("\"{}\": \"{}\"", key, value.to_str().unwrap()))
        .collect();
    let mut header = "{".to_owned() + &headers.join(", ") + "}";
    let headers = serde_json::from_str::<serde_json::Value>(&header).unwrap();
    header.truncate(98);
    let log = Log::new(headers["x-forwarded-for"].to_string(), header, log);
    let app_state = state.lock().await;
    let query = format!(
        "INSERT INTO logdetails (date, time, level, host_adr, header, message, reten_date) VALUES ('{}', '{}', '{}', '{}', '{}', '{}', '{}')",
        log.date, log.time, log.level, log.address, log.header, log.message, log.retention
    );
    let mut conn = app_state.db_pool.get_conn().unwrap();
    conn.query_drop(query).unwrap();
    tracing::debug!("Inserting log: {:?}", log);
    (StatusCode::CREATED, Json(json!({})))
}
