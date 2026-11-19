//! The connection to the bridge (ADR 0002, ADR 0009).

use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use mort_proto::{ErrorObject, Message, PROTOCOL_VERSION, Request, RequestId};

/// How long to wait for a reply before giving up.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(15);

/// Why a command failed, and which exit code that means (ADR 0009 §6).
#[derive(Debug)]
pub enum Failure {
    /// The bridge answered with an error. Exit 1.
    Bridge(ErrorObject),
    /// The bridge could not be reached or stopped answering. Exit 3.
    NoBridge(String),
    /// Something else went wrong on this side. Exit 1.
    Local(String),
}

impl Failure {
    /// The process exit code.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Failure::Bridge(_) | Failure::Local(_) => 1,
            Failure::NoBridge(_) => 3,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Bridge(e) => write!(f, "{e}"),
            Failure::NoBridge(m) | Failure::Local(m) => write!(f, "{m}"),
        }
    }
}

/// The socket named by `$TMPDIR/mort/current`.
///
/// # Errors
///
/// [`Failure::NoBridge`] if no bridge has recorded itself.
pub fn current_socket() -> Result<PathBuf, Failure> {
    let current = std::env::temp_dir().join("mort").join("current");
    std::fs::read_to_string(&current)
        .map(|s| PathBuf::from(s.trim()))
        .map_err(|_| no_bridge(&format!("{} does not exist", current.display())))
}

fn no_bridge(why: &str) -> Failure {
    Failure::NoBridge(format!(
        "no bridge running ({why}). Start REAPER with the bridge installed (`mort install`)"
    ))
}

/// A connection to the bridge.
pub struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next: i64,
    path: PathBuf,
}

impl Client {
    /// Connects and checks the protocol version with `bridge.hello`.
    ///
    /// # Errors
    ///
    /// [`Failure::NoBridge`] if nothing answers at `path`;
    /// [`Failure::Bridge`] if the versions do not match.
    pub fn connect(path: &Path) -> Result<(Client, Value), Failure> {
        let writer = UnixStream::connect(path)
            .map_err(|e| no_bridge(&format!("cannot connect to {}: {e}", path.display())))?;
        writer
            .set_read_timeout(Some(REPLY_TIMEOUT))
            .map_err(|e| Failure::Local(e.to_string()))?;
        let reader = BufReader::new(
            writer
                .try_clone()
                .map_err(|e| Failure::Local(e.to_string()))?,
        );
        let mut c = Client {
            writer,
            reader,
            next: 1,
            path: path.to_owned(),
        };
        let hello = c.call(
            "bridge.hello",
            json!({ "protocol": [PROTOCOL_VERSION.0, PROTOCOL_VERSION.1] }),
        )?;
        Ok((c, hello))
    }

    /// Calls a method and waits for its reply. Notifications that arrive
    /// meanwhile are skipped.
    ///
    /// # Errors
    ///
    /// [`Failure::Bridge`] for an error reply; [`Failure::NoBridge`] if the
    /// bridge goes away or does not answer in time.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, Failure> {
        self.call_within(method, params, REPLY_TIMEOUT)
    }

    /// Calls a method, waiting up to `timeout` for the reply. Renders hold
    /// REAPER's main thread, so waiting on a job needs longer than usual.
    ///
    /// # Errors
    ///
    /// As [`Client::call`].
    pub fn call_within(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, Failure> {
        self.writer
            .set_read_timeout(Some(timeout))
            .map_err(|e| Failure::Local(e.to_string()))?;
        let id = RequestId::Number(self.next);
        self.next += 1;
        let line = Message::Request(Request::new(id.clone(), method, params)).to_line();
        self.writer
            .write_all(line.as_bytes())
            .and_then(|()| self.writer.write_all(b"\n"))
            .map_err(|e| self.lost(&e))?;
        loop {
            let mut reply = String::new();
            match self.reader.read_line(&mut reply) {
                Ok(0) => return Err(no_bridge("the bridge closed the connection")),
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(Failure::NoBridge(format!(
                        "the bridge at {} did not answer `{method}` within {} s. REAPER runs \
                         mort only when it is idle: close any open dialog in REAPER and try again",
                        self.path.display(),
                        timeout.as_secs()
                    )));
                }
                Err(e) => return Err(self.lost(&e)),
            }
            match Message::parse(&reply) {
                Ok(Message::Response(r)) if r.id.as_ref() == Some(&id) => {
                    return r.outcome.map_err(Failure::Bridge);
                }
                Ok(Message::Response(r)) if r.id.is_none() => {
                    return Err(Failure::Bridge(r.outcome.err().unwrap_or_else(|| {
                        ErrorObject::internal("a reply with no id and no error")
                    })));
                }
                // Notifications, and replies to other requests.
                Ok(_) => {}
                Err(e) => {
                    return Err(Failure::Local(format!(
                        "the bridge sent something unreadable: {e}"
                    )));
                }
            }
        }
    }

    fn lost(&self, e: &io::Error) -> Failure {
        no_bridge(&format!(
            "lost the connection to {}: {e}",
            self.path.display()
        ))
    }
}
