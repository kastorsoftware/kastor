pub mod auth;
pub mod client;
pub mod crypto;
pub mod invite;
pub mod text_parse;
pub mod tl;
// Generated from schema.txt; clippy's style lints do not apply to protocol
// names and mechanically expanded TL constructors.
#[allow(clippy::all)]
pub mod tl_gen;
pub mod transport;

pub use client::is_fatal_session_error;
pub use client::is_network_error;

// mtproto service-layer constructor IDs.
// these belong to the transport/session protocol (mtproto.tl), not the API
// schema (schema.txt), so they are not produced by codegen. naming them here
// keeps client.rs free of bare hex literals.
pub mod service_ctors {
    pub const RPC_RESULT: u32 = 0xf35c6d01;
    pub const RPC_ERROR: u32 = 0x2144ca19;
    pub const MSG_CONTAINER: u32 = 0x73f1f8dc;
    pub const GZIP_PACKED: u32 = 0x3072cfa1;
    pub const BAD_SERVER_SALT: u32 = 0xedab447b;
    pub const BAD_MSG_NOTIFICATION: u32 = 0xa7eff811;
    pub const MSGS_ACK: u32 = 0x62d6b459;
    pub const NEW_SESSION_CREATED: u32 = 0x9ec20908;
    pub const UPDATES: u32 = 0x74ae4240;
    pub const UPDATES_COMBINED: u32 = 0x725b04c3;
    pub const UPDATE_SHORT: u32 = 0x78d4dec1;
    pub const UPDATE_SHORT_MESSAGE: u32 = 0x313bc7f8;
}
