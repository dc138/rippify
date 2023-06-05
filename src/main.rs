use async_recursion::async_recursion;
use colored::Colorize;
use librespot_audio::{AudioDecrypt, AudioFile};
use librespot_core::{
    authentication::Credentials, config::SessionConfig, session::Session, spotify_id::SpotifyId,
    Error, FileId,
};

use getopts::{Fail, Options};
use librespot_metadata::{audio::AudioFileFormat, Album, Artist, Metadata, Playlist, Track};
use oggvorbismeta::{replace_comment_header, CommentHeader, VorbisComments};
use regex::Regex;
use std::{
    collections::{HashSet, VecDeque},
    env, fmt,
    io::{Cursor, Read},
    path::Path,
    process::exit,
};
use tokio::{
    fs::{create_dir_all, File},
    io::copy,
};

#[tokio::main]
async fn main() {
    let opts = match parse_opts() {
        Ok(opts) => opts,
        Err(err) => {
            println!("{}: {}", "error".red().bold(), err.to_string().bold());
            exit(1);
        }
    };

    let credentials = Credentials::with_password(&opts.user, &opts.pass);
    let session_config = SessionConfig::default();

    let session = Session::new(session_config, None);

    match session.connect(credentials, false).await {
        Ok(_) => {
            println!(
                "{} Logged in as: {}",
                "=>".green().bold(),
                &opts.user.bright_blue()
            );
        }
        Err(err) => {
            println!(
                "{}: cannot log in: {}",
                "error".red().bold(),
                err.to_string().to_lowercase()
            );
            exit(1);
        }
    };

    println!("\n{} Input resources:", "=>".green().bold());

    let input_resources: Vec<_> = opts
        .input
        .iter()
        .map(|line| get_resource_from_line(line))
        .filter(|x| {
            if let Err(line) = x {
                println!(
                    "{}: unrecognized input: {}, skipping...",
                    " -> warning".yellow().bold(),
                    line.bold()
                );
                false
            } else {
                let res = x.as_ref().unwrap();
                println!(
                    " {} {}: {}",
                    "->".yellow().bold(),
                    res.kind,
                    &res.id.to_base62().unwrap()
                );
                true
            }
        })
        .map(|x| x.unwrap())
        .collect();

    let mut track_ids = HashSet::<SpotifyId>::new();

    for res in &input_resources {
        match res.get_tracks(&session).await {
            Ok(tracks) => track_ids.extend(tracks),
            Err(err) => {
                println!(
                    "{}: cannot get {} metadata: {}, skipping...",
                    "warning".yellow().bold(),
                    res.kind,
                    err
                );
            }
        }
    }

    if track_ids.is_empty() {
        println!(
            "\n{}: didn't get any tracks, aborting...",
            "error".red().bold()
        );
        exit(0);
    }

    println!(
        "\n{} Parsed {} tracks:",
        "=>".green().bold(),
        track_ids.len().to_string().bold()
    );

    let mut tracks_completed: usize = 0;
    let mut tracks_existing: usize = 0;

    for track_id in &track_ids {
        print!(" {} ", "->".yellow().bold());

        let (track, track_file_id) = match get_track_from_id(&session, track_id).await {
            Ok((track, file_id)) => {
                if track.id.to_base62().unwrap() != track_id.to_base62().unwrap() {
                    println!(
                        "{} ({} alt. {})",
                        track.name.bold(),
                        track.id.to_base62().unwrap(),
                        track_id.to_base62().unwrap()
                    );
                } else {
                    println!("{} ({})", track.name.bold(), track.id.to_base62().unwrap());
                }

                (track, file_id)
            }
            Err(e) => {
                println!("{} ({})", "??".bold(), track_id.to_base62().unwrap());
                println!(
                    "   - {}: cannot get track from id: {}, skipping...",
                    "warning".yellow().bold(),
                    e,
                );
                continue;
            }
        };

        let track_output_path = opts
            .format
            .clone()
            .replace("{author}", &track.artists.first().unwrap().name) // NOTE: using the first found artist as the "main" artist
            .replace("{album}", &track.album.name)
            .replace("{name}", &track.name.as_str().replace('/', " "))
            .replace("{ext}", "ogg");

        if Path::new(&track_output_path).exists() {
            println!(
                "   - {}: output file \"{}\" already exists, skipping...",
                "note".bright_blue().bold(),
                track_output_path
            );
            tracks_existing += 1;
            continue;
        }

        let slice_pos = match track_output_path.rfind('/') {
            Some(pos) => pos,
            None => {
                println!(
                    "{}: invalid format string {}, aborting...",
                    "error".red().bold(),
                    opts.format.bold()
                );
                exit(1);
            }
        };

        let track_folder_path = &track_output_path[..slice_pos + 1];

        if create_dir_all(track_folder_path).await.is_err() {
            print!(
                "   - {}: cannot create folders: {}, aborting...",
                "warning".yellow().bold(),
                track_folder_path
            );
            exit(1);
        }

        let track_file_key = match session.audio_key().request(track.id, track_file_id).await {
            Ok(key) => key,
            Err(err) => {
                println!(
                    "   - {}: cannot get audio key: {:?}, skipping",
                    "warning".yellow().bold(),
                    err
                );
                continue;
            }
        };

        let mut track_buffer = Vec::<u8>::new();
        let mut track_buffer_decrypted = Vec::<u8>::new();

        println!("   - getting encrypted audio file");

        let mut track_file_audio = match AudioFile::open(&session, track_file_id, 40).await {
            Ok(audio) => audio,
            Err(err) => {
                println!(
                    "   - {}: cannot get audio file: {:?}, skipping",
                    "warning".yellow().bold(),
                    err
                );
                continue;
            }
        };

        match track_file_audio.read_to_end(&mut track_buffer) {
            Ok(_) => {}
            Err(err) => {
                println!(
                    "   - {}: cannot get track file audio: {}, skipping",
                    "warning".yellow().bold(),
                    err
                );
                continue;
            }
        };

        println!("   - decrypting audio");

        match AudioDecrypt::new(Some(track_file_key), &track_buffer[..])
            .read_to_end(&mut track_buffer_decrypted)
        {
            Ok(_) => {}
            Err(err) => {
                println!(
                    "   - {}: cannot decrypt audio file: {}, skipping",
                    "warning".yellow().bold(),
                    err
                );
                continue;
            }
        };

        println!("   - writing output file");

        let track_file_cursor = Cursor::new(&track_buffer_decrypted[0xa7..]);
        let mut track_comments = CommentHeader::new();

        track_comments.set_vendor("Ogg");

        track_comments.add_tag_single("title", &track.name);
        track_comments.add_tag_single("album", &track.album.name);

        track
            .artists
            .iter()
            .for_each(|artist| track_comments.add_tag_single("artist", &artist.name));

        let mut track_file_out = replace_comment_header(track_file_cursor, track_comments);

        let mut track_file_write = File::create(&track_output_path).await.unwrap();
        match copy(&mut track_file_out, &mut track_file_write).await {
            Ok(_) => {
                println!("   - wrote \"{}\"", track_output_path);
            }
            Err(err) => {
                println!(
                    "   - {}: cannot write {}: {}, skipping...",
                    "warning".yellow().bold(),
                    track_output_path,
                    err
                );
                continue;
            }
        };

        tracks_completed += 1;
    }

    println!("\n{} Processed tracks: ", "=>".green().bold(),);

    println!(
        " {} {} error",
        "->".yellow().bold(),
        track_ids.len() - tracks_completed - tracks_existing
    );

    println!(
        " {} {} already downloaded",
        "->".yellow().bold(),
        tracks_existing
    );

    println!(" {} {} new", "->".yellow().bold(), tracks_completed);

    println!(
        " {} {} total processed",
        "->".yellow().bold(),
        track_ids.len()
    )
}

struct UserParams {
    user: String,
    pass: String,
    format: String,
    input: Vec<String>,
}

fn parse_opts() -> Result<UserParams, Fail> {
    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();

    opts.optflag("h", "help", "print the help menu");

    opts.optopt("u", "user", "user login name, required", "USER");
    opts.optopt("p", "pass", "user password, required", "PASS");
    opts.optopt(
        "f",
        "format",
        "output format to use. {author}/{album}/{name}.{ext} is used by default. Available format specifiers are: {author}, {album}, {name} and {ext}. Note that when tracks have more that one author, {author} will evaluate only to main one (track metadata will still we written correctly).",
        "FMT",
    );

    let matches = opts.parse(&args[1..])?;
    let input = matches.free.clone();

    if matches.opt_present("h")
        || !matches.opt_present("u")
        || !matches.opt_present("p")
        || input.is_empty()
    {
        print_usage(&program, opts);
        exit(0);
    }

    let format = if let Some(format) = matches.opt_str("f") {
        format
    } else {
        "{author}/{album}/{name}.{ext}".to_owned()
    };

    let user = matches.opt_str("u").unwrap();
    let pass = matches.opt_str("p").unwrap();

    Ok(UserParams {
        user,
        pass,
        format,
        input,
    })
}

fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} [OPTIONS] URIs...", program);
    print!("{}", opts.usage(&brief));
}

enum ResourceKind {
    Track,
    Playlist,
    Album,
    Artist,
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ResourceKind::Track => write!(f, "track"),
            ResourceKind::Playlist => write!(f, "playlist"),
            ResourceKind::Album => write!(f, "album"),
            ResourceKind::Artist => write!(f, "artist"),
        }
    }
}

impl ResourceKind {
    fn to_url_regex(&self) -> Regex {
        Regex::new(&format!(
            r"^(http(s)?://)?open\.spotify\.com/{}/([[:alnum:]]{{22}})$",
            self
        ))
        .unwrap()
    }

    fn to_uri_regex(&self) -> Regex {
        Regex::new(&format!(r"^spotify:{}:([[:alnum:]]{{22}})$", self)).unwrap()
    }
}

struct InputResource {
    kind: ResourceKind,
    id: SpotifyId,
}

impl InputResource {
    #[async_recursion]
    async fn get_tracks(&self, session: &Session) -> Result<Vec<SpotifyId>, Error> {
        let mut tracks: Vec<SpotifyId> = Vec::new();

        match self.kind {
            ResourceKind::Track => {
                tracks.push(self.id);
            }
            ResourceKind::Playlist => {
                let playlist = Playlist::get(session, &self.id).await?;
                tracks.extend(playlist.tracks());
            }
            ResourceKind::Album => {
                let album = Album::get(session, &self.id).await?;
                tracks.extend(album.tracks());
            }
            ResourceKind::Artist => {
                let artist = Artist::get(session, &self.id).await?;

                for album_group in artist.albums.0 {
                    for album in album_group.0 .0 {
                        tracks.extend(
                            InputResource {
                                kind: ResourceKind::Album,
                                id: album,
                            }
                            .get_tracks(session)
                            .await?,
                        );
                    }
                }

                for album_group in artist.singles.0 {
                    for album in album_group.0 .0 {
                        tracks.extend(
                            InputResource {
                                kind: ResourceKind::Album,
                                id: album,
                            }
                            .get_tracks(session)
                            .await?,
                        );
                    }
                }
            }
        }

        Ok(tracks)
    }
}

fn get_resource_from_line(line: &str) -> Result<InputResource, &str> {
    if let Some(id) = is_resource(line, ResourceKind::Track) {
        Ok(InputResource {
            kind: ResourceKind::Track,
            id,
        })
    //
    } else if let Some(id) = is_resource(line, ResourceKind::Album) {
        Ok(InputResource {
            kind: ResourceKind::Album,
            id,
        })
    //
    } else if let Some(id) = is_resource(line, ResourceKind::Playlist) {
        Ok(InputResource {
            kind: ResourceKind::Playlist,
            id,
        })
    //
    } else if let Some(id) = is_resource(line, ResourceKind::Artist) {
        Ok(InputResource {
            kind: ResourceKind::Artist,
            id,
        })
    //
    } else {
        Err(line)
    }
}

fn is_resource(line: &str, res: ResourceKind) -> Option<SpotifyId> {
    if let Some(captures) = res
        .to_url_regex()
        .captures(line)
        .or(res.to_uri_regex().captures(line))
    {
        let id_str = captures.iter().last().unwrap().unwrap().as_str();
        let id = SpotifyId::from_base62(id_str).unwrap();

        Some(id)
    //
    } else {
        None
    }
}

async fn get_track_from_id(session: &Session, id: &SpotifyId) -> Result<(Track, FileId), Error> {
    let mut track_ids = VecDeque::<SpotifyId>::new();
    track_ids.push_back(id.to_owned());

    while let Some(id) = track_ids.pop_front() {
        let track = Track::get(session, &id).await?;

        match None
            .or(track.files.get_key_value(&AudioFileFormat::OGG_VORBIS_320))
            .or(track.files.get_key_value(&AudioFileFormat::OGG_VORBIS_160))
            .or(track.files.get_key_value(&AudioFileFormat::OGG_VORBIS_96))
        {
            Some(format) => return Ok((track.to_owned(), format.1.to_owned())),
            None => track_ids.extend(track.alternatives.0),
        };
    }

    Err(Error::internal("cannot find a suitable track"))
}
