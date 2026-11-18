//! mort's protocol: the types the CLI and the bridge exchange.
//!
//! - [`ids`]: REAPER GUIDs wrapped in a distinct type per kind of element
//!   (ADR 0003 §1).
//! - [`position`]: musical and clock positions, their parsing, and
//!   conversion to project seconds through a [`position::TempoMap`]
//!   (ADR 0003 §4).
//! - [`rpc`]: the JSON-RPC 2.0 envelope, `batch`, and mort's error shape
//!   (ADR 0002).

pub mod ids;
pub mod position;
pub mod rpc;

pub use ids::{EnvelopeId, FxId, Guid, GuidError, ItemId, TakeId, TrackId};
pub use position::{Position, PositionError, Range, SteppedTempoMap, TempoChange, TempoMap};
pub use rpc::{
    BatchCall, BatchParams, ErrorData, ErrorKind, ErrorObject, Message, Notification,
    PROTOCOL_VERSION, Request, RequestId, Response,
};
