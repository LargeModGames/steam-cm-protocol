pub mod auth {
    pub mod credentials;
    pub mod qr;
}
pub mod client;
pub mod connection;
pub mod emsg;
pub mod eresult;
pub mod error;
pub mod message;
pub mod protobuf {
    include!(concat!(env!("OUT_DIR"), "/_includes.rs"));
}
pub mod serverlist;
pub mod service_method;
pub mod token;
pub mod transport {
    pub mod websocket;
}

pub use client::{AuthEvent, AuthMethod, GuardKind, LoggedOn, SteamClient};
pub use error::{Error, Result};
