//! `mort view`: the map view in a browser (ADR 0012 §9, ticket 0020).
//!
//! A small HTTP/1.1 server in the CLI process, on `127.0.0.1` only. It
//! serves the page (embedded in the binary) and passes a few calls through
//! to the bridge:
//!
//! - `GET /` — the page;
//! - `GET /events` — server-sent events from `map.watch`: `change` when the
//!   map or thresholds change, `cursor` while the transport plays,
//!   `bridge` when the bridge goes away or comes back;
//! - `GET /api/view?start&end&step` — `map.view`;
//! - `GET /api/track?track&start&end` — `map.track`;
//! - `GET /api/thresholds`, `POST /api/thresholds` — `thresholds.get`
//!   and `thresholds.set`, the only change the page can make.
//!
//! Every request must carry the token printed with the URL (`?t=`), and a
//! `Host` naming the loopback address and this port. Anything else gets
//! 403, so other web pages cannot read the map or change thresholds, even
//! through DNS rebinding. No dependency is needed for so little HTTP.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Args;
use serde_json::{Value, json};

use crate::client::{self, Client, Failure};

/// The page, embedded.
const PAGE: &str = include_str!("../assets/view.html");

/// The first port tried.
const PORT: u16 = 7317;

/// `mort view` arguments.
#[derive(Args)]
pub struct View {
    /// The port to try first; the next free one is used if it is taken. 0
    /// picks any free port.
    #[arg(long, default_value_t = PORT)]
    port: u16,
    /// Open the page in the browser.
    #[arg(long)]
    open: bool,
}

/// Where the bridge is: the `--socket` given, or whatever `current` says
/// at the time.
#[derive(Clone)]
struct Bridge(Option<PathBuf>);

impl Bridge {
    fn connect(&self) -> Result<Client, Failure> {
        let path = match &self.0 {
            Some(p) => p.clone(),
            None => client::current_socket()?,
        };
        Client::connect(&path).map(|(c, _)| c)
    }
}

/// A random token: 128 bits from the system, in hex.
fn token() -> String {
    let mut b = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut b);
    }
    if b == [0u8; 16] {
        // No /dev/urandom: fall back on time and process, still unguessable
        // enough for a page only this user can reach.
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        b = (n ^ (u128::from(std::process::id()) << 64)).to_le_bytes();
    }
    b.iter().fold(String::new(), |mut acc, x| {
        use std::fmt::Write as _;
        let _ = write!(acc, "{x:02x}");
        acc
    })
}

fn bind(port: u16) -> Result<TcpListener, Failure> {
    if port == 0 {
        return TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| Failure::Local(format!("cannot listen on 127.0.0.1: {e}")));
    }
    for p in port..port.saturating_add(20) {
        if let Ok(l) = TcpListener::bind(("127.0.0.1", p)) {
            return Ok(l);
        }
    }
    Err(Failure::Local(format!(
        "ports {port} to {} are all taken; pick another with --port",
        port.saturating_add(19)
    )))
}

/// Runs `mort view` until it is stopped.
///
/// # Errors
///
/// If it cannot listen.
pub fn run(socket: Option<PathBuf>, args: &View) -> Result<(), Failure> {
    let listener = bind(args.port)?;
    let port = listener
        .local_addr()
        .map_err(|e| Failure::Local(e.to_string()))?
        .port();
    let tok = Arc::new(token());
    let url = format!("http://127.0.0.1:{port}/?t={tok}");
    println!("{url}");
    println!("the map view is served until you stop this with Ctrl-C");
    let _ = std::io::stdout().flush();
    if args.open {
        let _ = std::process::Command::new("open").arg(&url).spawn();
    }
    let bridge = Bridge(socket);
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let (bridge, tok) = (bridge.clone(), Arc::clone(&tok));
        let _ = std::thread::Builder::new()
            .name("mort-view".into())
            .spawn(move || {
                let _ = serve(stream, &bridge, &tok, port);
            });
    }
    Ok(())
}

/// One request: method, path, query and body.
struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = |x: u8| char::from(x).to_digit(16);
                match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push(u8::try_from(hi * 16 + lo).unwrap_or(b'?'));
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_request(stream: &TcpStream) -> std::io::Result<Option<Request>> {
    let mut r = BufReader::new(stream);
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let target = parts.next().unwrap_or("/").to_owned();
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if headers.len() > 100 {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        .min(1 << 16);
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    let (path, q) = target.split_once('?').unwrap_or((&target, ""));
    let query = q
        .split('&')
        .filter_map(|kv| kv.split_once('=').or(Some((kv, ""))))
        .filter(|(k, _)| !k.is_empty())
        .map(|(k, v)| (unescape(k), unescape(v)))
        .collect();
    Ok(Some(Request {
        method,
        path: path.to_owned(),
        query,
        headers,
        body,
    }))
}

fn respond(mut s: &TcpStream, status: &str, kind: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    s.write_all(body)?;
    s.flush()
}

fn respond_json(s: &TcpStream, status: &str, v: &Value) -> std::io::Result<()> {
    respond(s, status, "application/json", v.to_string().as_bytes())
}

/// Whether the request is for this server on the loopback address: a
/// rebound DNS name would carry its own `Host`.
fn host_ok(req: &Request, port: u16) -> bool {
    req.headers
        .get("host")
        .is_some_and(|h| h == &format!("127.0.0.1:{port}") || h == &format!("localhost:{port}"))
}

fn serve(stream: TcpStream, bridge: &Bridge, tok: &str, port: u16) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let Some(req) = read_request(&stream)? else {
        return Ok(());
    };
    if !host_ok(&req, port) || req.query.get("t").map(String::as_str) != Some(tok) {
        return respond(&stream, "403 Forbidden", "text/plain", b"forbidden\n");
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => respond(
            &stream,
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes(),
        ),
        ("GET", "/events") => events(stream, bridge, &req),
        ("GET", "/api/view") => pass(
            &stream,
            bridge,
            "map.view",
            params(&req, &["start", "end", "step"]),
        ),
        ("GET", "/api/track") => {
            let mut p = params(&req, &["start", "end"]);
            if let Some(t) = req.query.get("track") {
                p["track"] = json!(t);
            }
            pass(&stream, bridge, "map.track", p)
        }
        ("GET", "/api/thresholds") => pass(&stream, bridge, "thresholds.get", Value::Null),
        ("POST", "/api/thresholds") => match serde_json::from_slice::<Value>(&req.body) {
            Ok(v) if v.is_object() => pass(&stream, bridge, "thresholds.set", v),
            _ => respond_json(
                &stream,
                "400 Bad Request",
                &json!({ "error": "send a JSON object" }),
            ),
        },
        _ => respond(&stream, "404 Not Found", "text/plain", b"not found\n"),
    }
}

/// Numeric query parameters as RPC params.
fn params(req: &Request, names: &[&str]) -> Value {
    let mut p = json!({});
    for n in names {
        if let Some(v) = req.query.get(*n).and_then(|v| v.parse::<f64>().ok()) {
            p[*n] = json!(v);
        }
    }
    p
}

/// Passes a call to the bridge and its result, or error, back as JSON.
fn pass(s: &TcpStream, bridge: &Bridge, method: &str, params: Value) -> std::io::Result<()> {
    let r = bridge.connect().and_then(|mut c| c.call(method, params));
    match r {
        Ok(v) => respond_json(s, "200 OK", &v),
        Err(Failure::Bridge(e)) => {
            respond_json(s, "422 Unprocessable Entity", &json!({ "error": e }))
        }
        Err(Failure::NoBridge(m)) => respond_json(
            s,
            "503 Service Unavailable",
            &json!({ "error": m, "bridge": false }),
        ),
        Err(Failure::Local(m)) => {
            respond_json(s, "500 Internal Server Error", &json!({ "error": m }))
        }
    }
}

fn send_event(mut s: &TcpStream, event: &str, data: &Value) -> std::io::Result<()> {
    write!(s, "event: {event}\ndata: {data}\n\n")?;
    s.flush()
}

/// `/events`: the change feed as server-sent events, for as long as the
/// page listens. While the transport plays, watches are short so that the
/// cursor moves; otherwise they wait up to 15 s. If the bridge goes away,
/// the page is told and the server tries again every 2 s.
fn events(mut s: TcpStream, bridge: &Bridge, req: &Request) -> std::io::Result<()> {
    s.set_read_timeout(None)?;
    write!(
        s,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\n\r\n"
    )?;
    let mut since: u64 = req
        .query
        .get("since")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut up = true;
    let mut playing = false;
    loop {
        let mut c = match bridge.connect() {
            Ok(c) => c,
            Err(e) => {
                if up {
                    send_event(&s, "bridge", &json!({ "up": false, "why": e.to_string() }))?;
                    up = false;
                }
                std::thread::sleep(Duration::from_secs(2));
                // Notice a page that has gone away.
                send_event(&s, "ping", &json!({}))?;
                continue;
            }
        };
        if !up {
            send_event(&s, "bridge", &json!({ "up": true }))?;
            up = true;
        }
        loop {
            let wait = if playing { 200 } else { 15_000 };
            let r = c.call_within(
                "map.watch",
                json!({ "since": since, "timeout_ms": wait }),
                Duration::from_millis(wait + 5000),
            );
            let Ok(v) = r else { break };
            let version = v["version"].as_u64().unwrap_or(since);
            let changed = version > since || v["reset"] == true;
            playing = v["playing"] == true;
            if changed {
                send_event(&s, "change", &v)?;
                since = version;
            } else if playing {
                send_event(
                    &s,
                    "cursor",
                    &json!({ "position": v["position"], "playing": true }),
                )?;
            } else {
                send_event(
                    &s,
                    "cursor",
                    &json!({ "position": v["position"], "playing": false }),
                )?;
            }
        }
    }
}
