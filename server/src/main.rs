//! The server process: read the configuration, open the database, accept.

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use planner_server::http::{self, Response};
use planner_server::{check_token, Server};

fn main() {
    // `--health` connects to the configured address and asks. It exists
    // because the image is built from a slim base with no curl and no wget,
    // and because a healthcheck that needed the bearer token would mean
    // handing the secret to Docker as well.
    if std::env::args().any(|argument| argument == "--health") {
        std::process::exit(health());
    }

    if let Err(message) = run() {
        eprintln!("planner-server: {message}");
        std::process::exit(1);
    }
}

/// Everything the process needs, from the environment.
///
/// No defaults for the secret or the database, on purpose: a server that
/// invents a token starts successfully and protects nothing.
struct Settings {
    address: String,
    token: String,
    database: String,
}

fn settings() -> Result<Settings, String> {
    let address = std::env::var("PLANNER_ADDR").unwrap_or_else(|_| "0.0.0.0:8083".to_string());
    let token = std::env::var("PLANNER_TOKEN").map_err(|_| "set PLANNER_TOKEN".to_string())?;
    check_token(&token)?;

    // Assembled from parts rather than taken as one URL so that the password
    // is one environment variable among several and never lands in a process
    // listing as part of a connection string.
    let host = std::env::var("PLANNER_DB_HOST").map_err(|_| "set PLANNER_DB_HOST".to_string())?;
    let port = std::env::var("PLANNER_DB_PORT").unwrap_or_else(|_| "5432".to_string());
    let name = std::env::var("PLANNER_DB_NAME").unwrap_or_else(|_| "planner".to_string());
    let user = std::env::var("PLANNER_DB_USER").unwrap_or_else(|_| "planner".to_string());
    let password =
        std::env::var("PLANNER_DB_PASSWORD").map_err(|_| "set PLANNER_DB_PASSWORD".to_string())?;

    let database = format!("host={host} port={port} dbname={name} user={user} password={password}");

    Ok(Settings {
        address,
        token,
        database,
    })
}

fn run() -> Result<(), String> {
    let settings = settings()?;

    let client = postgres::Client::connect(&settings.database, postgres::NoTls)
        .map_err(|error| format!("could not reach the database: {error}"))?;

    let listener = TcpListener::bind(&settings.address)
        .map_err(|error| format!("could not listen on {}: {error}", settings.address))?;

    // Printed rather than logged: Container Manager's Log tab is unreliable,
    // and this is the line that says the process got past its configuration.
    println!("planner-server listening on {}", settings.address);

    let server = Arc::new(Server::new(client, settings.token));

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let server = Arc::clone(&server);
        // A thread per connection. One person's three machines on a timer is
        // a few requests a minute, and a thread that lives for one request is
        // simpler to reason about than anything that pools.
        std::thread::spawn(move || serve(&server, stream));
    }

    Ok(())
}

fn serve(server: &Server, mut stream: TcpStream) {
    let response = match http::read_request(&stream) {
        Ok(request) => server.handle(&request),
        Err(error) => Response::text(400, &error.0),
    };

    if let Err(error) = http::write_response(&mut stream, &response) {
        // The client hung up. Worth a line, not worth taking anything down.
        eprintln!("planner-server: could not reply: {error}");
    }
    let _ = stream.flush();
}

/// Ask the running server whether it is up. Zero means yes.
fn health() -> i32 {
    let address = std::env::var("PLANNER_ADDR").unwrap_or_else(|_| "0.0.0.0:8083".to_string());
    // Whatever it binds, it is reachable from inside the container on
    // loopback, and `0.0.0.0` is not an address to connect to.
    let port = address.rsplit(':').next().unwrap_or("8083");

    let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{port}")) else {
        return 1;
    };
    if write!(stream, "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n").is_err() {
        return 1;
    }

    let mut reply = String::new();
    use std::io::Read;
    if stream.read_to_string(&mut reply).is_err() {
        return 1;
    }
    i32::from(!reply.starts_with("HTTP/1.1 200"))
}
