use tokio::sync::oneshot;

use crate::{
    connection::{Connection, ConnectionState},
    emsg::EMsg,
    error::{Error, Result},
    friends::ProtocolGame,
    message::Packet,
    protobuf::{CMsgProtoBufHeader, CPlayerGetOwnedGamesRequest, CPlayerGetOwnedGamesResponse},
    service_method::ServiceMethod,
};

/// Send the GetOwnedGames request and return the response receiver.
/// The caller should release the connection lock before awaiting the receiver.
pub async fn start_get_owned_games(
    connection: &Connection,
    state: &ConnectionState,
) -> Result<oneshot::Receiver<Result<Packet>>> {
    let steamid = state
        .steamid
        .ok_or(Error::MissingField("steamid not set in connection state"))?;

    let method = ServiceMethod::new("Player.GetOwnedGames#1");
    let request = CPlayerGetOwnedGamesRequest {
        steamid: Some(steamid),
        include_appinfo: Some(true),
        include_played_free_games: Some(true),
        include_free_sub: Some(false),
        ..Default::default()
    };

    connection
        .send_request(
            EMsg::ServiceMethodCallFromClient,
            CMsgProtoBufHeader {
                steamid: state.steamid,
                client_sessionid: state.client_session_id,
                target_job_name: Some(method.target_job_name),
                ..Default::default()
            },
            &request,
        )
        .await
}

/// Decode a GetOwnedGames response packet into a list of games.
pub fn decode_owned_games(packet: Packet) -> Result<Vec<ProtocolGame>> {
    let response: CPlayerGetOwnedGamesResponse = packet.decode_body()?;

    let mut games: Vec<ProtocolGame> = response
        .games
        .into_iter()
        .filter_map(|g| {
            let appid = g.appid?;
            if appid <= 0 {
                return None;
            }
            Some(ProtocolGame {
                appid: appid as u32,
                name: g.name.unwrap_or_default(),
                playtime_forever: g.playtime_forever.unwrap_or(0),
                rtime_last_played: g.rtime_last_played.unwrap_or(0),
                img_icon_url: g.img_icon_url,
            })
        })
        .collect();

    games.sort_by(|a, b| b.playtime_forever.cmp(&a.playtime_forever));
    Ok(games)
}
