use librespot_core::{
    authentication::Credentials, config::SessionConfig, session::Session, spotify_id::SpotifyId,
};

use debug_print::debug_println as dprintln;
use getopts::Options;
use lazy_regex::regex;
use librespot_metadata::{Album, Metadata, Playlist};
use std::{collections::HashSet, env};

fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} [OPTIONS] URIs...", program);
    print!("{}", opts.usage(&brief));
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();

    opts.optflag("h", "help", "print the help menu");
    opts.optopt("u", "user", "user login name, required", "USER");
    opts.optopt("p", "pass", "user password, required", "PASS");

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => {
            panic!("{}", f.to_string())
        }
    };

    let input = matches.free.clone();

    if matches.opt_present("h")
        || !matches.opt_present("u")
        || !matches.opt_present("p")
        || input.len() == 0
    {
        print_usage(&program, opts);
        return;
    }

    let user = matches.opt_str("u").unwrap();
    let pass = matches.opt_str("p").unwrap();

    dprintln!("user: {}, pass: {}", &user, &pass);
    let credentials = Credentials::with_password(&user, &pass);

    let track_uri = regex!(r"^spotify:track:([[:alnum:]]{22})$");
    let track_url = regex!(r"^(http(s)?://)?open\.spotify\.com/track/([[:alnum:]]{22})$");
    let pl_uri = regex!(r"^spotify:playlist:([[:alnum:]]{22})$");
    let pl_url = regex!(r"^(http(s)?://)?open\.spotify\.com/playlist/([[:alnum:]]{22})$");
    let album_uri = regex!(r"^spotify:album:([[:alnum:]]{22})$");
    let album_url = regex!(r"^(http(s)?://)?open\.spotify\.com/album/([[:alnum:]]{22})$");

    let session_config = SessionConfig::default();
    let session = Session::connect(session_config, credentials, None, false)
        .await
        .unwrap();

    let mut tracks: HashSet<SpotifyId> = HashSet::new();

    dprintln!("input tracks:");

    for line in &input {
        if let Some(captures) = track_uri.captures(&line).or(track_url.captures(&line)) {
            let id_str = captures.iter().last().unwrap().unwrap().as_str();
            let id = SpotifyId::from_base62(&id_str).unwrap();
            dprintln!("read track: {}", &id_str);
            tracks.insert(id);
        } else if let Some(captures) = pl_uri.captures(&line).or(pl_url.captures(&line)) {
            let id_str = captures.iter().last().unwrap().unwrap().as_str();
            let id = SpotifyId::from_base62(&id_str).unwrap();
            dprintln!("read playlist: {}", &id_str);
            let playlist = Playlist::get(&session.0, id)
                .await
                .expect("error getting playlist metadata");

            for track in playlist.tracks {
                tracks.insert(track);
            }
        } else if let Some(captures) = album_uri.captures(&line).or(album_url.captures(&line)) {
            let id_str = captures.iter().last().unwrap().unwrap().as_str();
            let id = SpotifyId::from_base62(&id_str).unwrap();
            dprintln!("read album: {}", &id_str);
            let album = Album::get(&session.0, id)
                .await
                .expect("error getting album metadata");

            for track in album.tracks {
                tracks.insert(track);
            }
        } else {
            panic!("unkown input: {}", line);
        }
    }

    dprintln!("parsed {} tracks:", tracks.len());

    for track in &tracks {
        dprintln!("got track: {}", track.to_uri().unwrap());
    }
}
