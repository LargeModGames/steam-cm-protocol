//! 1-on-1 friend chat over the modern unified `FriendMessages.*` service.
//!
//! Mirrors the `library.rs` / `service_method.rs` pattern: outbound requests go through
//! `call_authed` (`ServiceMethodCallFromClient`, EMsg 151 → `ServiceMethodResponse` 147), and
//! incoming messages arrive unsolicited as a `ServiceMethodSendToClient` (EMsg 152) server push.
//! No new EMsg constants and no subscribe call are needed — Steam pushes friend messages to any
//! logged-on client automatically.

use crate::{
    connection::{Connection, ConnectionState},
    emsg::EMsg,
    error::Result,
    friends::FriendsEvent,
    message::Packet,
    protobuf::{
        CFriendMessagesGetRecentMessagesRequest, CFriendMessagesGetRecentMessagesResponse,
        CFriendMessagesIncomingMessageNotification, CFriendMessagesSendMessageRequest,
        CFriendMessagesSendMessageResponse,
    },
    service_method::{ServiceMethod, call_authed},
};

/// `EChatEntryType` values we use. The enum isn't present in any compiled proto, so the raw
/// ints are hardcoded (values per SteamKit `enums.steamd`).
pub const CHAT_ENTRY_TEXT: i32 = 1;
pub const CHAT_ENTRY_TYPING: i32 = 2;

/// Job name carried by the incoming-message server push (EMsg 152).
const INCOMING_MESSAGE_JOB: &str = "FriendMessagesClient.IncomingMessage#1";

/// A single 1-on-1 chat message, normalised for the UI/cache layers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    /// The conversation partner (the friend), regardless of who authored the message.
    pub steamid: u64,
    pub message: String,
    /// Steam `rtime32` server timestamp (Unix seconds).
    pub timestamp: u32,
    /// Disambiguates multiple messages sharing the same second.
    pub ordinal: u32,
    /// True when the local user authored this message.
    pub from_local: bool,
}

/// Send a text message to `steamid`.
pub async fn send_message(
    connection: &Connection,
    state: &ConnectionState,
    steamid: u64,
    message: String,
) -> Result<()> {
    let method = ServiceMethod::new("FriendMessages.SendMessage#1");
    let request = CFriendMessagesSendMessageRequest {
        steamid: Some(steamid),
        chat_entry_type: Some(CHAT_ENTRY_TEXT),
        message: Some(message),
        // Send the raw text verbatim; brackets render literally in the terminal.
        contains_bbcode: Some(false),
        ..Default::default()
    };
    // Body is ignored: the 147 response itself is the send confirmation.
    let _response: CFriendMessagesSendMessageResponse =
        call_authed(connection, state, &method, &request).await?;
    Ok(())
}

/// Send a typing indicator to `steamid` (best-effort).
pub async fn send_typing(
    connection: &Connection,
    state: &ConnectionState,
    steamid: u64,
) -> Result<()> {
    let method = ServiceMethod::new("FriendMessages.SendMessage#1");
    let request = CFriendMessagesSendMessageRequest {
        steamid: Some(steamid),
        chat_entry_type: Some(CHAT_ENTRY_TYPING),
        message: Some(String::new()),
        ..Default::default()
    };
    let _response: CFriendMessagesSendMessageResponse =
        call_authed(connection, state, &method, &request).await?;
    Ok(())
}

/// Fetch recent message history for the conversation with `steamid`, oldest first.
pub async fn get_recent_messages(
    connection: &Connection,
    state: &ConnectionState,
    steamid: u64,
) -> Result<Vec<ChatMessage>> {
    let method = ServiceMethod::new("FriendMessages.GetRecentMessages#1");
    let request = CFriendMessagesGetRecentMessagesRequest {
        steamid1: state.steamid,
        steamid2: Some(steamid),
        count: Some(50),
        ..Default::default()
    };
    let response: CFriendMessagesGetRecentMessagesResponse =
        call_authed(connection, state, &method, &request).await?;
    Ok(recent_to_messages(response, steamid, state.steamid))
}

/// Decode an incoming friend-message server push (EMsg 152). Returns `None` for any other 152
/// push (reactions, session notices, group-chat client pushes) so the run loop ignores them.
pub fn decode_incoming(packet: &Packet) -> Option<FriendsEvent> {
    if packet.emsg != EMsg::ServiceMethodSendToClient.raw()
        || packet.target_job_name() != Some(INCOMING_MESSAGE_JOB)
    {
        return None;
    }
    let notification = packet
        .decode_body::<CFriendMessagesIncomingMessageNotification>()
        .ok()?;
    // `steamid_friend` is always the partner, even for cross-session echoes of our own sends.
    let partner = notification.steamid_friend?;
    match notification.chat_entry_type.unwrap_or(0) {
        CHAT_ENTRY_TEXT => Some(FriendsEvent::IncomingMessage(ChatMessage {
            steamid: partner,
            message: notification.message.unwrap_or_default(),
            timestamp: notification.rtime32_server_timestamp.unwrap_or(0),
            ordinal: notification.ordinal.unwrap_or(0),
            // Direction comes from `local_echo`, never from comparing steamids.
            from_local: notification.local_echo.unwrap_or(false),
        })),
        CHAT_ENTRY_TYPING => Some(FriendsEvent::TypingNotification { steamid: partner }),
        _ => None,
    }
}

/// The low 32 bits of a SteamID64 are the account id used in `GetRecentMessages` rows.
fn account_id_of(steamid: Option<u64>) -> Option<u32> {
    steamid.map(|s| (s & 0xFFFF_FFFF) as u32)
}

fn recent_to_messages(
    response: CFriendMessagesGetRecentMessagesResponse,
    partner: u64,
    self_steamid: Option<u64>,
) -> Vec<ChatMessage> {
    let self_account = account_id_of(self_steamid);
    let mut messages: Vec<ChatMessage> = response
        .messages
        .into_iter()
        .map(|m| ChatMessage {
            steamid: partner,
            message: m.message.unwrap_or_default(),
            timestamp: m.timestamp.unwrap_or(0),
            ordinal: m.ordinal.unwrap_or(0),
            from_local: m.accountid.is_some() && m.accountid == self_account,
        })
        .collect();
    // Steam does not guarantee ordering; sort oldest-first for display.
    messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.ordinal.cmp(&b.ordinal)));
    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{decode_frame, encode_message};
    use prost::Message;
    use crate::protobuf::{
        CMsgProtoBufHeader, c_friend_messages_get_recent_messages_response::FriendMessage,
    };

    const SELF_STEAMID: u64 = 76561198000000001;
    const PARTNER_STEAMID: u64 = 76561198000000002;

    fn incoming_packet(job: &str, notification: &CFriendMessagesIncomingMessageNotification) -> Packet {
        let header = CMsgProtoBufHeader {
            target_job_name: Some(job.to_owned()),
            ..Default::default()
        };
        let encoded = encode_message(EMsg::ServiceMethodSendToClient, &header, notification).unwrap();
        decode_frame(&encoded)
            .unwrap()
            .into_iter()
            .next()
            .expect("one packet")
    }

    #[test]
    fn send_request_roundtrips() {
        let request = CFriendMessagesSendMessageRequest {
            steamid: Some(PARTNER_STEAMID),
            chat_entry_type: Some(CHAT_ENTRY_TEXT),
            message: Some("hi there".to_owned()),
            contains_bbcode: Some(false),
            ..Default::default()
        };
        let bytes = request.encode_to_vec();
        let back = CFriendMessagesSendMessageRequest::decode(bytes.as_slice()).unwrap();
        assert_eq!(back.steamid, Some(PARTNER_STEAMID));
        assert_eq!(back.message.as_deref(), Some("hi there"));
        assert_eq!(back.chat_entry_type, Some(CHAT_ENTRY_TEXT));
    }

    #[test]
    fn decodes_incoming_text_message() {
        let notification = CFriendMessagesIncomingMessageNotification {
            steamid_friend: Some(PARTNER_STEAMID),
            chat_entry_type: Some(CHAT_ENTRY_TEXT),
            message: Some("hello".to_owned()),
            rtime32_server_timestamp: Some(1000),
            ordinal: Some(2),
            local_echo: Some(false),
            ..Default::default()
        };
        let packet = incoming_packet(INCOMING_MESSAGE_JOB, &notification);
        match decode_incoming(&packet) {
            Some(FriendsEvent::IncomingMessage(m)) => {
                assert_eq!(m.steamid, PARTNER_STEAMID);
                assert_eq!(m.message, "hello");
                assert_eq!(m.timestamp, 1000);
                assert_eq!(m.ordinal, 2);
                assert!(!m.from_local);
            }
            other => panic!("expected IncomingMessage, got {other:?}"),
        }
    }

    #[test]
    fn decodes_incoming_typing() {
        let notification = CFriendMessagesIncomingMessageNotification {
            steamid_friend: Some(PARTNER_STEAMID),
            chat_entry_type: Some(CHAT_ENTRY_TYPING),
            ..Default::default()
        };
        let packet = incoming_packet(INCOMING_MESSAGE_JOB, &notification);
        match decode_incoming(&packet) {
            Some(FriendsEvent::TypingNotification { steamid }) => assert_eq!(steamid, PARTNER_STEAMID),
            other => panic!("expected TypingNotification, got {other:?}"),
        }
    }

    #[test]
    fn ignores_unrelated_push_job() {
        let notification = CFriendMessagesIncomingMessageNotification {
            steamid_friend: Some(PARTNER_STEAMID),
            chat_entry_type: Some(CHAT_ENTRY_TEXT),
            message: Some("from a group".to_owned()),
            ..Default::default()
        };
        let packet = incoming_packet("ChatRoomClient.NotifyIncomingChatMessage#1", &notification);
        assert!(decode_incoming(&packet).is_none());
    }

    #[test]
    fn recent_messages_marks_local_author_and_sorts_oldest_first() {
        let self_account = (SELF_STEAMID & 0xFFFF_FFFF) as u32;
        let response = CFriendMessagesGetRecentMessagesResponse {
            messages: vec![
                FriendMessage {
                    accountid: Some(self_account),
                    timestamp: Some(200),
                    message: Some("my reply".to_owned()),
                    ordinal: Some(0),
                    ..Default::default()
                },
                FriendMessage {
                    accountid: Some(0xDEAD_BEEF),
                    timestamp: Some(100),
                    message: Some("their hello".to_owned()),
                    ordinal: Some(0),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let messages = recent_to_messages(response, PARTNER_STEAMID, Some(SELF_STEAMID));
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].message, "their hello");
        assert!(!messages[0].from_local);
        assert_eq!(messages[0].steamid, PARTNER_STEAMID);
        assert_eq!(messages[1].message, "my reply");
        assert!(messages[1].from_local);
    }
}
