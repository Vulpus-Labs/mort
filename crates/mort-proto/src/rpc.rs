//! The JSON-RPC 2.0 envelope and mort's error shape (ADR 0002).
//!
//! Messages travel one per line. A [`Request`] has an id and gets a
//! [`Response`]. A [`Notification`] has none and gets no reply; the bridge
//! uses notifications for `project.changed` and `job.finished`.
//!
//! The `batch` method takes [`BatchParams`]: calls run in one dispatcher
//! tick, as one undo point if they edit (ADR 0002 §2, ADR 0004 §1). Its
//! result is an array with one entry per call. JSON-RPC's own array
//! batches are not used.

use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

/// The protocol version, as (major, minor). Clients refuse a different
/// major version (ADR 0002 §5).
pub const PROTOCOL_VERSION: (u32, u32) = (0, 1);

/// The `jsonrpc` member: always `"2.0"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct V2;

impl Serialize for V2 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for V2 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s == "2.0" {
            Ok(V2)
        } else {
            Err(D::Error::custom(format!(
                "jsonrpc must be \"2.0\", not {s:?}"
            )))
        }
    }
}

/// A request id: a number or a string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    /// A numeric id.
    Number(i64),
    /// A string id.
    String(String),
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestId::Number(n) => write!(f, "{n}"),
            RequestId::String(s) => write!(f, "{s:?}"),
        }
    }
}

/// A call that expects a response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Always `"2.0"`.
    pub jsonrpc: V2,
    /// Echoed in the response.
    pub id: RequestId,
    /// The method, for example `tracks.list`.
    pub method: String,
    /// The parameters; `null` when there are none.
    #[serde(default)]
    pub params: Value,
}

impl Request {
    /// A request with the given id, method and parameters.
    #[must_use]
    pub fn new(id: RequestId, method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: V2,
            id,
            method: method.into(),
            params,
        }
    }
}

/// A one-way message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    /// Always `"2.0"`.
    pub jsonrpc: V2,
    /// The method, for example `project.changed`.
    pub method: String,
    /// The parameters; `null` when there are none.
    #[serde(default)]
    pub params: Value,
}

impl Notification {
    /// A notification with the given method and parameters.
    #[must_use]
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: V2,
            method: method.into(),
            params,
        }
    }
}

/// The reply to a [`Request`].
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    /// The request's id; `None` when the request could not be read.
    pub id: Option<RequestId>,
    /// The result, or the error.
    pub outcome: Result<Value, ErrorObject>,
}

impl Response {
    /// A successful response.
    #[must_use]
    pub fn ok(id: RequestId, result: Value) -> Self {
        Self {
            id: Some(id),
            outcome: Ok(result),
        }
    }

    /// An error response.
    #[must_use]
    pub fn err(id: Option<RequestId>, error: ErrorObject) -> Self {
        Self {
            id,
            outcome: Err(error),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct RawResponse {
    jsonrpc: V2,
    id: Option<RequestId>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<ErrorObject>,
}

/// A `result` member that is present is `Some`, even when it is `null`.
fn present_value<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

impl Serialize for Response {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (result, error) = match &self.outcome {
            Ok(v) => (Some(v.clone()), None),
            Err(e) => (None, Some(e.clone())),
        };
        RawResponse {
            jsonrpc: V2,
            id: self.id.clone(),
            result,
            error,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Response {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = RawResponse::deserialize(d)?;
        let outcome = match (raw.result, raw.error) {
            (Some(v), None) => Ok(v),
            (None, Some(e)) => Err(e),
            _ => {
                return Err(D::Error::custom(
                    "a response has exactly one of `result` and `error`",
                ));
            }
        };
        Ok(Response {
            id: raw.id,
            outcome,
        })
    }
}

/// Any message on the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// A call that expects a response.
    Request(Request),
    /// A one-way message.
    Notification(Notification),
    /// A reply.
    Response(Response),
}

impl Message {
    /// Reads one line of the wire format.
    ///
    /// # Errors
    ///
    /// A JSON-RPC parse error or invalid-request error, ready to send back.
    pub fn parse(line: &str) -> Result<Message, ErrorObject> {
        let value: Value =
            serde_json::from_str(line).map_err(|e| ErrorObject::parse_error(&e.to_string()))?;
        let Value::Object(obj) = &value else {
            return Err(ErrorObject::invalid_request("a message is a JSON object"));
        };
        let invalid = |e: serde_json::Error| ErrorObject::invalid_request(&e.to_string());
        if obj.contains_key("method") {
            if obj.contains_key("id") {
                serde_json::from_value(value)
                    .map(Message::Request)
                    .map_err(invalid)
            } else {
                serde_json::from_value(value)
                    .map(Message::Notification)
                    .map_err(invalid)
            }
        } else if obj.contains_key("result") || obj.contains_key("error") {
            serde_json::from_value(value)
                .map(Message::Response)
                .map_err(invalid)
        } else {
            Err(ErrorObject::invalid_request(
                "a message has `method` (a call) or `result`/`error` (a reply)",
            ))
        }
    }

    /// Writes the message as one line, without the newline.
    ///
    /// # Panics
    ///
    /// Never in practice: protocol types always serialise.
    #[must_use]
    pub fn to_line(&self) -> String {
        let r = match self {
            Message::Request(m) => serde_json::to_string(m),
            Message::Notification(m) => serde_json::to_string(m),
            Message::Response(m) => serde_json::to_string(m),
        };
        r.expect("protocol types always serialise")
    }
}

/// The kinds of error mort reports (ADR 0002 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The element named does not exist (any more).
    NotFound,
    /// The project changed since the client read it (ADR 0004 §2).
    Stale,
    /// The request is well formed but cannot be done as asked.
    Invalid,
    /// Edits are held (ADR 0004 §4), or the bridge is busy.
    Busy,
    /// REAPER refused or failed.
    Reaper,
}

impl ErrorKind {
    /// The JSON-RPC error code for this kind, in the server-error range.
    #[must_use]
    pub fn code(self) -> i32 {
        match self {
            ErrorKind::NotFound => -32001,
            ErrorKind::Stale => -32002,
            ErrorKind::Invalid => -32003,
            ErrorKind::Busy => -32004,
            ErrorKind::Reaper => -32005,
        }
    }

    /// The name used on the wire and printed by the CLI.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            ErrorKind::NotFound => "not_found",
            ErrorKind::Stale => "stale",
            ErrorKind::Invalid => "invalid",
            ErrorKind::Busy => "busy",
            ErrorKind::Reaper => "reaper",
        }
    }
}

/// The `data` member of a mort error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorData {
    /// What kind of error it is.
    pub kind: ErrorKind,
    /// The id or name of the element the error is about, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// One sentence saying what to do.
    pub fix: String,
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorObject {
    /// The JSON-RPC code.
    pub code: i32,
    /// What went wrong.
    pub message: String,
    /// mort's details; absent for JSON-RPC's own protocol errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

impl ErrorObject {
    /// A mort error of the given kind.
    #[must_use]
    pub fn new(
        kind: ErrorKind,
        message: impl Into<String>,
        subject: Option<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self {
            code: kind.code(),
            message: message.into(),
            data: Some(ErrorData {
                kind,
                subject,
                fix: fix.into(),
            }),
        }
    }

    /// The line could not be read as JSON.
    #[must_use]
    pub fn parse_error(detail: &str) -> Self {
        Self::protocol(-32700, format!("not JSON: {detail}"))
    }

    /// The JSON is not a valid message.
    #[must_use]
    pub fn invalid_request(detail: &str) -> Self {
        Self::protocol(-32600, format!("not a valid message: {detail}"))
    }

    /// No such method.
    #[must_use]
    pub fn method_not_found(method: &str) -> Self {
        Self::protocol(-32601, format!("no method `{method}`"))
    }

    /// The parameters do not fit the method.
    #[must_use]
    pub fn invalid_params(method: &str, detail: &str) -> Self {
        Self::protocol(-32602, format!("bad parameters for `{method}`: {detail}"))
    }

    /// The bridge failed unexpectedly.
    #[must_use]
    pub fn internal(detail: &str) -> Self {
        Self::protocol(-32603, format!("internal error: {detail}"))
    }

    fn protocol(code: i32, message: String) -> Self {
        Self {
            code,
            message,
            data: None,
        }
    }

    /// The mort error kind, if this is a mort error.
    #[must_use]
    pub fn kind(&self) -> Option<ErrorKind> {
        self.data.as_ref().map(|d| d.kind)
    }
}

impl fmt::Display for ErrorObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.data {
            None => write!(f, "{}", self.message),
            Some(d) => {
                write!(f, "{}: {}", d.kind.name(), self.message)?;
                if let Some(s) = &d.subject {
                    write!(f, " ({s})")?;
                }
                write!(f, ". {}", d.fix)
            }
        }
    }
}

impl std::error::Error for ErrorObject {}

/// One call inside a `batch`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchCall {
    /// The method.
    pub method: String,
    /// Its parameters; `null` when there are none.
    #[serde(default)]
    pub params: Value,
}

/// Parameters of the `batch` method.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchParams {
    /// The calls, run in order.
    pub calls: Vec<BatchCall>,
    /// The undo label for the batch's edits, without the `mort: ` prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Refuse with `stale` unless the project's state count is this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_state: Option<u64>,
}

/// Helper for building parameter objects in tests and clients.
#[must_use]
pub fn params(pairs: &[(&str, Value)]) -> Value {
    Value::Object(
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect::<Map<_, _>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip(m: &Message) {
        let line = m.to_line();
        assert!(!line.contains('\n'));
        assert_eq!(&Message::parse(&line).unwrap(), m, "{line}");
    }

    #[test]
    fn requests_notifications_and_responses_round_trip() {
        round_trip(&Message::Request(Request::new(
            RequestId::Number(7),
            "tracks.list",
            Value::Null,
        )));
        round_trip(&Message::Request(Request::new(
            RequestId::String("a".into()),
            "fx.params",
            json!({"fx": "{0A1B2C3D-4E5F-6071-8293-A4B5C6D7E8F9}"}),
        )));
        round_trip(&Message::Notification(Notification::new(
            "project.changed",
            json!({"state": 12, "by_mort": false}),
        )));
        round_trip(&Message::Response(Response::ok(
            RequestId::Number(7),
            json!([1, 2]),
        )));
        round_trip(&Message::Response(Response::ok(
            RequestId::Number(8),
            Value::Null,
        )));
        round_trip(&Message::Response(Response::err(
            Some(RequestId::Number(9)),
            ErrorObject::new(
                ErrorKind::Stale,
                "the project changed since it was read",
                None,
                "read it again and decide again",
            ),
        )));
        round_trip(&Message::Response(Response::err(
            None,
            ErrorObject::parse_error("eof"),
        )));
    }

    #[test]
    fn batch_round_trips() {
        let p = BatchParams {
            calls: vec![
                BatchCall {
                    method: "track.add".into(),
                    params: json!({"name": "bass"}),
                },
                BatchCall {
                    method: "fx.add".into(),
                    params: Value::Null,
                },
            ],
            summary: Some("add bass".into()),
            expect_state: Some(41),
        };
        let req = Message::Request(Request::new(
            RequestId::Number(1),
            "batch",
            serde_json::to_value(&p).unwrap(),
        ));
        round_trip(&req);
        let Message::Request(r) = Message::parse(&req.to_line()).unwrap() else {
            panic!()
        };
        let back: BatchParams = serde_json::from_value(r.params).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn error_shape_on_the_wire() {
        let e = ErrorObject::new(
            ErrorKind::NotFound,
            "no such track",
            Some("{0A1B2C3D-4E5F-6071-8293-A4B5C6D7E8F9}".into()),
            "list tracks with `mort tracks`",
        );
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], -32001);
        assert_eq!(v["data"]["kind"], "not_found");
        assert_eq!(
            v["data"]["subject"],
            "{0A1B2C3D-4E5F-6071-8293-A4B5C6D7E8F9}"
        );
        assert_eq!(v["data"]["fix"], "list tracks with `mort tracks`");
        assert_eq!(
            e.to_string(),
            "not_found: no such track ({0A1B2C3D-4E5F-6071-8293-A4B5C6D7E8F9}). \
             list tracks with `mort tracks`"
        );
    }

    #[test]
    fn rejects_bad_messages() {
        assert_eq!(Message::parse("{").unwrap_err().code, -32700);
        assert_eq!(Message::parse("[1]").unwrap_err().code, -32600);
        assert_eq!(Message::parse("{}").unwrap_err().code, -32600);
        let wrong_version = r#"{"jsonrpc":"1.0","id":1,"method":"x"}"#;
        assert!(
            Message::parse(wrong_version)
                .unwrap_err()
                .message
                .contains("\"2.0\"")
        );
        let both = r#"{"jsonrpc":"2.0","id":1,"result":1,"error":{"code":1,"message":"m"}}"#;
        assert!(
            Message::parse(both)
                .unwrap_err()
                .message
                .contains("exactly one")
        );
    }

    #[test]
    fn missing_params_are_null() {
        let m = Message::parse(r#"{"jsonrpc":"2.0","id":"x","method":"bridge.hello"}"#).unwrap();
        let Message::Request(r) = m else { panic!() };
        assert_eq!(r.params, Value::Null);
        assert_eq!(r.id.to_string(), "\"x\"");
    }
}
