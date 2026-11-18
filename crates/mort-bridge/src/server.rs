//! The socket server (ADR 0002 §1).
//!
//! One thread accepts connections. Each connection gets a reader thread,
//! which parses lines and passes requests to the main thread, and a writer
//! thread, which writes whatever the main thread sends it. Lines that are
//! not valid messages are answered by the reader directly. None of these
//! threads call REAPER.

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use mort_proto::{Message, Response};

use crate::dispatch::Inbound;

/// The directory sockets live in: `$TMPDIR/mort`.
#[must_use]
pub fn socket_dir() -> PathBuf {
    std::env::temp_dir().join("mort")
}

/// A running server. Dropping it stops accepting, disconnects clients and
/// removes the socket (and `current`, if it still names this socket).
pub struct Server {
    path: PathBuf,
    current: PathBuf,
    stop: Arc<AtomicBool>,
    streams: Arc<Mutex<Vec<UnixStream>>>,
    accept: Option<JoinHandle<()>>,
}

impl Server {
    /// Listens at `<dir>/<name>.sock`, records it in `<dir>/current`, and
    /// sends what clients say to `inbound`.
    ///
    /// # Errors
    ///
    /// If the directory or socket cannot be created.
    pub fn start(dir: &Path, name: &str, inbound: Sender<Inbound>) -> io::Result<Server> {
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        remove_dead_sockets(dir);
        let path = dir.join(format!("{name}.sock"));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let current = dir.join("current");
        let tmp = dir.join(format!(".current.{name}"));
        fs::write(&tmp, path.to_string_lossy().as_bytes())?;
        fs::rename(&tmp, &current)?;

        let stop = Arc::new(AtomicBool::new(false));
        let streams = Arc::new(Mutex::new(Vec::new()));
        let accept = {
            let stop = Arc::clone(&stop);
            let streams = Arc::clone(&streams);
            thread::Builder::new()
                .name("mort-accept".into())
                .spawn(move || accept_loop(&listener, &stop, &streams, &inbound))?
        };
        Ok(Server {
            path,
            current,
            stop,
            streams,
            accept: Some(accept),
        })
    }

    /// The socket's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept thread.
        let _ = UnixStream::connect(&self.path);
        if let Some(h) = self.accept.take() {
            let _ = h.join();
        }
        if let Ok(streams) = self.streams.lock() {
            for s in streams.iter() {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
        }
        let _ = fs::remove_file(&self.path);
        if fs::read_to_string(&self.current).is_ok_and(|c| Path::new(&c) == self.path) {
            let _ = fs::remove_file(&self.current);
        }
    }
}

fn accept_loop(
    listener: &UnixListener,
    stop: &AtomicBool,
    streams: &Mutex<Vec<UnixStream>>,
    inbound: &Sender<Inbound>,
) {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let Ok(stream) = stream else { continue };
        let conn = NEXT.fetch_add(1, Ordering::Relaxed);
        let (Ok(read), Ok(keep)) = (stream.try_clone(), stream.try_clone()) else {
            continue;
        };
        if let Ok(mut s) = streams.lock() {
            s.push(keep);
        }
        let (tx, rx) = mpsc::channel::<String>();
        if inbound
            .send(Inbound::Opened {
                conn,
                tx: tx.clone(),
            })
            .is_err()
        {
            return;
        }
        let mut write = stream;
        let _ = thread::Builder::new()
            .name(format!("mort-write-{conn}"))
            .spawn(move || {
                for line in rx {
                    if write
                        .write_all(line.as_bytes())
                        .and_then(|()| write.write_all(b"\n"))
                        .and_then(|()| write.flush())
                        .is_err()
                    {
                        return;
                    }
                }
            });
        let inbound = inbound.clone();
        let _ = thread::Builder::new()
            .name(format!("mort-read-{conn}"))
            .spawn(move || read_loop(conn, read, &tx, &inbound));
    }
}

fn read_loop(conn: u64, stream: UnixStream, tx: &Sender<String>, inbound: &Sender<Inbound>) {
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        match Message::parse(&line) {
            Ok(Message::Request(request)) => {
                if inbound.send(Inbound::Request { conn, request }).is_err() {
                    return;
                }
            }
            // Clients have nothing to notify or answer; ignore.
            Ok(Message::Notification(_) | Message::Response(_)) => {}
            Err(e) => {
                let _ = tx.send(Message::Response(Response::err(None, e)).to_line());
            }
        }
    }
    let _ = inbound.send(Inbound::Closed { conn });
}

/// Removes `<pid>.sock` files whose REAPER is no longer running.
fn remove_dead_sockets(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "sock") && UnixStream::connect(&p).is_err() {
            let _ = fs::remove_file(&p);
        }
    }
}
