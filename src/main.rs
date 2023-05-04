use librespot_core::{
    authentication::Credentials, config::SessionConfig, session::Session, spotify_id::SpotifyId,
};

use debug_print::debug_println as dprintln;
use getopts::Options;
use lazy_regex::regex;
use librespot_metadata::{Album, FileFormat, Metadata, Playlist, Track};
use std::{collections::HashSet, env, process::exit};

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

    let session = match Session::connect(session_config, credentials, None, false).await {
        Ok(session) => session,
        Err(err) => {
            println!("Cannot log in: {}", err.to_string().to_lowercase());
            exit(1);
        }
    };

    let mut track_ids: HashSet<SpotifyId> = HashSet::new();

    println!("Input resources:");

    for line in &input {
        if let Some(captures) = track_uri.captures(&line).or(track_url.captures(&line)) {
            let id_str = captures.iter().last().unwrap().unwrap().as_str();
            let id = SpotifyId::from_base62(&id_str).unwrap();

            println!("  track: {}", &id_str);

            track_ids.insert(id);
        } else if let Some(captures) = pl_uri.captures(&line).or(pl_url.captures(&line)) {
            let id_str = captures.iter().last().unwrap().unwrap().as_str();
            let id = SpotifyId::from_base62(&id_str).unwrap();

            println!("  playlist: {}", &id_str);

            let playlist = match Playlist::get(&session.0, id).await {
                Ok(playlist) => playlist,
                Err(err) => {
                    println!("    error getting playlist metadata: {:?}, skipping", err);
                    continue;
                }
            };

            for track in playlist.tracks {
                track_ids.insert(track);
            }
        } else if let Some(captures) = album_uri.captures(&line).or(album_url.captures(&line)) {
            let id_str = captures.iter().last().unwrap().unwrap().as_str();
            let id = SpotifyId::from_base62(&id_str).unwrap();

            println!("  album: {}", &id_str);

            let album = match Album::get(&session.0, id).await {
                Ok(album) => album,
                Err(err) => {
                    println!("    error getting album metadata: {:?}, skipping", err);
                    continue;
                }
            };

            for track in album.tracks {
                track_ids.insert(track);
            }
        } else {
            println!("  unkown input: {}, skipping", line);
        }
    }

    if track_ids.len() == 0 {
        println!("Didn't get any tracks");
        exit(0);
    }

    println!("Parsed {} tracks:", track_ids.len());

    for track_id in track_ids {
        println!("  track: {}", track_id.to_uri().unwrap());

        let track = Track::get(&session.0, track_id).await.unwrap();

        if !track.available {
            println!("    unavailable, skipping");
            continue;
        }

        let track_file_id = match track
            .files
            .get(&FileFormat::OGG_VORBIS_320)
            .or(track.files.get(&FileFormat::OGG_VORBIS_160))
            .or(track.files.get(&FileFormat::OGG_VORBIS_96))
        {
            Some(format) => format,
            None => {
                println!("    no suitable format found, skipping");
                continue;
            }
        };

        //let track_file_key = session.0.audio_key().
    }
}
