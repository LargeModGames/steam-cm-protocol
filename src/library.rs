use crate::{
    connection::{Connection, ConnectionState},
    error::{Error, Result},
    friends::ProtocolGame,
    protobuf::{CPlayerGetOwnedGamesRequest, CPlayerGetOwnedGamesResponse},
    service_method::{ServiceMethod, call_authed},
};

pub async fn get_owned_games(
    connection: &Connection,
    state: &ConnectionState,
) -> Result<Vec<ProtocolGame>> {
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

    let response: CPlayerGetOwnedGamesResponse =
        call_authed(connection, state, &method, &request).await?;

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
