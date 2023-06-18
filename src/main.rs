use async_recursion::async_recursion;
use colored::Colorize;
use lewton::header as lhr;
use librespot_audio as lsa;
use librespot_core as lsc;
use librespot_core::authentication as lsc_auth;
use librespot_metadata as lsm;
use librespot_metadata::audio as lsm_audio;
use lsm::Metadata;
use std::collections as coll;
use std::env;
use std::fmt;
use std::fs;
use std::io;
use std::io::Read;
use std::path;
use std::process as proc;

static VERSION: &str = "0.2.0";

#[tokio::main]
async fn main() {
    let opts = match parse_opts() {
        Ok(opts) => opts,
        Err(err) => {
            println!("{}: {}", "error".red().bold(), err.to_string().bold());
            proc::exit(1);
        }
    };

    let credentials = lsc_auth::Credentials::with_password(&opts.user, &opts.pass);
    let session_config = lsc::SessionConfig::default();

    let session = lsc::Session::new(session_config, None);

    match session.connect(credentials, false).await {
        Ok(_) => {
            println!("{} Logged in as: {}", "=>".green().bold(), &opts.user.bright_blue());
        }
        Err(err) => {
            println!(
                "{}: cannot log in: {}",
                "error".red().bold(),
                err.to_string().to_lowercase()
            );
            proc::exit(1);
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

    let mut input_tracks = coll::HashSet::<lsc::SpotifyId>::new();

    for res in &input_resources {
        match res.get_tracks(&session).await {
            Ok(tracks) => input_tracks.extend(tracks),
            Err(err) => {
                println!(
                    "{}: cannot get metadata for {} {}: {}, skipping...",
                    "warning".yellow().bold(),
                    res.kind,
                    res.id.to_base62().unwrap(),
                    err
                );
            }
        }
    }

    if input_tracks.is_empty() {
        println!("\n{}: didn't get any tracks, aborting...", "error".red().bold());
        proc::exit(0);
    }

    println!(
        "\n{} Parsed {} tracks:",
        "=>".green().bold(),
        input_tracks.len().to_string().bold()
    );

    let mut num_completed: usize = 0;
    let mut num_existing: usize = 0;

    for track_id in &input_tracks {
        print!(" {} ", "->".yellow().bold());

        let (track, file_id) = match get_track_from_id(&session, track_id).await {
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
            Err(err) => {
                println!("{} ({})", "??".bold(), track_id.to_base62().unwrap());
                println!(
                    "   - {}: cannot get track from id: {}, skipping...",
                    "warning".yellow().bold(),
                    err,
                );
                continue;
            }
        };

        let output_file = opts.format.parse_output_format(&track);

        if path::Path::new(&output_file.file).exists() {
            println!(
                "   - {}: output file \"{}\" already exists, skipping...",
                "note".bright_blue().bold(),
                output_file.file
            );

            num_existing += 1;
            continue;
        }

        let buffer = match track_download(&track, &file_id, &session).await {
            Ok(buffer) => buffer,
            Err(err) => {
                match err.kind {
                    TrackDownloadErrorKind::AudioKey => {
                        println!(
                            "   - {}: cannot get audio key: {}, skipping...",
                            "warning".yellow().bold(),
                            err.error
                        );
                    }
                    TrackDownloadErrorKind::AudioFile => {
                        println!(
                            "   - {}: cannot get audio file: {}, skipping...",
                            "warning".yellow().bold(),
                            err.error
                        );
                    }
                    TrackDownloadErrorKind::TrackFile => {
                        println!(
                            "   - {}: cannot get track file audio: {}, skipping...",
                            "warning".yellow().bold(),
                            err.error
                        );
                    }
                    TrackDownloadErrorKind::Decrypt => {
                        println!(
                            "   - {}: cannot decrypt audio file: {}, skipping...",
                            "warning".yellow().bold(),
                            err.error
                        );
                    }
                };
                continue;
            }
        };

        let buffer_tags = match track_add_metadata_tags(buffer, &track) {
            Ok(buf) => buf,
            Err(err) => {
                match err.kind {
                    TagsWriteErrorKind::Read => {
                        print!(
                            "   - {}: cannot read ogg packet: {}, skipping...",
                            "warning".yellow().bold(),
                            err.error
                        );
                    }
                    TagsWriteErrorKind::Write => {
                        print!(
                            "   - {}: cannot write ogg packet: {}, skipping...",
                            "warning".yellow().bold(),
                            err.error
                        );
                    }
                    TagsWriteErrorKind::Header => {
                        print!(
                            "   - {}: cannot create comment header packet: {}, skipping...",
                            "warning".yellow().bold(),
                            err.error
                        );
                    }
                }
                continue;
            }
        };

        match track_write(buffer_tags, output_file) {
            Ok(output) => {
                println!("   - wrote \"{}\"", output);
                num_completed += 1;
            }
            Err(err) => {
                match err.kind {
                    TrackWriteErrorKind::FolderCreate => {
                        print!(
                            "   - {}: cannot create output folders: {}, skipping...",
                            "warning".yellow().bold(),
                            err.error
                        );
                    }
                    TrackWriteErrorKind::FileCreate => {
                        println!(
                            "   - {}: cannot create output file: {}, skipping...",
                            "warning".yellow().bold(),
                            err.error
                        );
                    }
                    TrackWriteErrorKind::FileWrite => {
                        println!(
                            "   - {}: cannot write output file: {}, skipping...",
                            "warning".yellow().bold(),
                            err.error
                        );
                    }
                };
                continue;
            }
        };
    }

    println!("\n{} Processed tracks: ", "=>".green().bold(),);

    println!(
        " {} {} error",
        "->".yellow().bold(),
        input_tracks.len() - num_completed - num_existing
    );

    println!(" {} {} already downloaded", "->".yellow().bold(), num_existing);

    println!(" {} {} new", "->".yellow().bold(), num_completed);

    println!(" {} {} total processed", "->".yellow().bold(), input_tracks.len())
}

struct UserParams {
    user: String,
    pass: String,
    format: OutputFormat,
    input: Vec<String>,
}

fn parse_opts() -> Result<UserParams, getopts::Fail> {
    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut opts = getopts::Options::new();

    opts.optflag("h", "help", "print the help menu");
    opts.optflag("v", "version", "show copyright and version information");

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

    if matches.opt_present("v") {
        print_version();
        proc::exit(0);
    }

    if matches.opt_present("h") || !matches.opt_present("u") || !matches.opt_present("p") || input.is_empty() {
        print_usage(&program, opts);
        proc::exit(0);
    }

    let format = OutputFormat {
        format_string: matches
            .opt_str("f")
            .unwrap_or("{author}/{album}/{name}.{ext}".to_owned()),
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

fn print_usage(program: &str, opts: getopts::Options) {
    let brief = format!("Usage: {} [OPTIONS] URIs...", program);
    print!("{}", opts.usage(&brief));
}

fn print_version() {
    println!("rippify version {}\n", VERSION);
    println!(
        "Copyright (C) 2023 Antonio de Haro. \n\
        This program is distributed under the MIT license, see the attatched LICENSE.txt file for terms and conditions. \n\
        This software is provided without any warranty of any kind. \n\
        Copyright atributions for any third party code included are provided in the attatched COPYRIGHT.md file."
    );
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
    fn to_url_regex(&self) -> regex::Regex {
        regex::Regex::new(&format!(
            r"^(http(s)?://)?open\.spotify\.com/{}/([[:alnum:]]{{22}})$",
            self
        ))
        .unwrap()
    }

    fn to_uri_regex(&self) -> regex::Regex {
        regex::Regex::new(&format!(r"^spotify:{}:([[:alnum:]]{{22}})$", self)).unwrap()
    }
}

struct InputResource {
    kind: ResourceKind,
    id: lsc::SpotifyId,
}

impl InputResource {
    #[async_recursion]
    async fn get_tracks(&self, session: &lsc::Session) -> Result<Vec<lsc::SpotifyId>, librespot_core::error::Error> {
        let mut tracks: Vec<lsc::SpotifyId> = Vec::new();

        match self.kind {
            ResourceKind::Track => {
                tracks.push(self.id);
            }
            ResourceKind::Playlist => {
                let playlist = lsm::Playlist::get(session, &self.id).await?;
                tracks.extend(playlist.tracks());
            }
            ResourceKind::Album => {
                let album = lsm::Album::get(session, &self.id).await?;
                tracks.extend(album.tracks());
            }
            ResourceKind::Artist => {
                let artist = lsm::Artist::get(session, &self.id).await?;

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

fn is_resource(line: &str, res: ResourceKind) -> Option<lsc::SpotifyId> {
    if let Some(captures) = res.to_url_regex().captures(line).or(res.to_uri_regex().captures(line)) {
        let id_str = captures.iter().last().unwrap().unwrap().as_str();
        let id = lsc::SpotifyId::from_base62(id_str).unwrap();

        Some(id)
    //
    } else {
        None
    }
}

async fn get_track_from_id(
    session: &lsc::Session,
    id: &lsc::SpotifyId,
) -> Result<(lsm::Track, lsc::FileId), librespot_core::error::Error> {
    let mut track_ids = coll::VecDeque::<lsc::SpotifyId>::new();
    track_ids.push_back(id.to_owned());

    while let Some(id) = track_ids.pop_front() {
        let track = lsm::Track::get(session, &id).await?;

        match None
            .or(track.files.get_key_value(&lsm_audio::AudioFileFormat::OGG_VORBIS_320))
            .or(track.files.get_key_value(&lsm_audio::AudioFileFormat::OGG_VORBIS_160))
            .or(track.files.get_key_value(&lsm_audio::AudioFileFormat::OGG_VORBIS_96))
        {
            Some(format) => return Ok((track.to_owned(), format.1.to_owned())),
            None => track_ids.extend(track.alternatives.0),
        };
    }

    Err(librespot_core::error::Error::not_found("cannot find a suitable track"))
}

struct OutputFormat {
    format_string: String,
}

#[derive(Debug)]
struct OutputFile {
    dir: Option<String>,
    file: String,
}

impl OutputFormat {
    fn parse_output_format(&self, track: &lsm::Track) -> OutputFile {
        let parsed = self
            .format_string
            .replace("{author}", &track.artists.first().unwrap().name) // NOTE: using the first found artist as the "main" artist
            .replace("{album}", &track.album.name)
            .replace("{name}", &track.name.as_str().replace('/', " "))
            .replace("{ext}", "ogg");

        OutputFile {
            dir: parsed.rfind('/').map(|split_pos| parsed[..=split_pos].to_owned()),
            file: parsed,
        }
    }
}

trait ProcessErrorKind {}

struct ProcessError<T: ProcessErrorKind> {
    kind: T,
    error: Box<dyn std::error::Error>,
}

enum TrackDownloadErrorKind {
    AudioKey,
    AudioFile,
    TrackFile,
    Decrypt,
}

impl ProcessErrorKind for TrackDownloadErrorKind {}
type TrackDownloadError = ProcessError<TrackDownloadErrorKind>;

async fn track_download(
    track: &lsm::Track,
    file_id: &lsc::FileId,
    session: &lsc::Session,
) -> Result<Vec<u8>, TrackDownloadError> {
    let track_file_key = session
        .audio_key()
        .request(track.id, *file_id)
        .await
        .map_err(|e| ProcessError {
            kind: TrackDownloadErrorKind::AudioKey,
            error: e.into(),
        })?;

    let mut track_buffer = Vec::<u8>::new();
    let mut track_buffer_decrypted = Vec::<u8>::new();

    let mut track_file_audio = lsa::AudioFile::open(session, *file_id, 40)
        .await
        .map_err(|e| ProcessError {
            kind: TrackDownloadErrorKind::AudioFile,
            error: e.into(),
        })?;

    track_file_audio
        .read_to_end(&mut track_buffer)
        .map_err(|e| ProcessError {
            kind: TrackDownloadErrorKind::TrackFile,
            error: e.into(),
        })?;

    lsa::AudioDecrypt::new(Some(track_file_key), &track_buffer[..])
        .read_to_end(&mut track_buffer_decrypted)
        .map_err(|e| ProcessError {
            kind: TrackDownloadErrorKind::Decrypt,
            error: e.into(),
        })?;

    Ok(track_buffer_decrypted[0xa7..].to_vec())
}

enum TrackWriteErrorKind {
    FolderCreate,
    FileCreate,
    FileWrite,
}

impl ProcessErrorKind for TrackWriteErrorKind {}
type TrackWriteError = ProcessError<TrackWriteErrorKind>;

fn track_write(track_buffer: Vec<u8>, output_file: OutputFile) -> Result<String, TrackWriteError> {
    if let Some(path) = output_file.dir {
        fs::create_dir_all(path).map_err(|e| TrackWriteError {
            kind: TrackWriteErrorKind::FolderCreate,
            error: e.into(),
        })?;
    }

    let mut file_write = fs::File::create(&output_file.file).map_err(|e| ProcessError {
        kind: TrackWriteErrorKind::FileCreate,
        error: e.into(),
    })?;

    io::copy(&mut track_buffer.as_slice(), &mut file_write).map_err(|e| ProcessError {
        kind: TrackWriteErrorKind::FileWrite,
        error: e.into(),
    })?;

    Ok(output_file.file)
}

fn track_add_metadata_tags(track_buffer: Vec<u8>, track: &lsm::Track) -> Result<Vec<u8>, TagsWriteError> {
    let mut metadata = lhr::CommentHeader {
        vendor: String::from("Ogg"),
        comment_list: Vec::new(),
    };

    metadata.comment_list.push((String::from("title"), track.name.clone()));
    metadata
        .comment_list
        .push((String::from("album"), track.album.name.clone()));

    metadata.comment_list.extend(
        track
            .artists
            .iter()
            .map(|artist| (String::from("artist"), artist.name.clone()))
            .collect::<Vec<_>>(),
    );

    replace_header_comment(&track_buffer, &metadata)
}

// Reverse implementation of https://github.com/RustAudio/lewton/blob/bb2955b717094b40260902cf2f8dd9c5ea62a84a/src/header.rs#L309
fn make_header_comment(header: &lhr::CommentHeader) -> Option<Vec<u8>> {
    let mut packet: Vec<u8> = vec![];

    // 'V' 'O' 'R' 'B' 'I' 'S'
    packet.extend([0x03, 0x76, 0x6F, 0x72, 0x62, 0x69, 0x73] as [u8; 7]);

    let vendor_buf = header.vendor.as_bytes();
    let vendor_len = TryInto::<u32>::try_into(vendor_buf.len()).ok()?.to_le_bytes();

    packet.extend(vendor_len);
    packet.extend(vendor_buf);

    let comments_len = TryInto::<u32>::try_into(header.comment_list.len()).ok()?.to_le_bytes();

    packet.extend(comments_len);

    for comment in &header.comment_list {
        let comment_buf = format!("{}={}", comment.0, comment.1);
        let comment_buf = comment_buf.as_bytes();
        let comment_len = TryInto::<u32>::try_into(comment_buf.len()).ok()?.to_le_bytes();

        packet.extend(comment_len);
        packet.extend(comment_buf);
    }

    packet.extend([0x01] as [u8; 1]);
    Some(packet)
}

enum TagsWriteErrorKind {
    Read,
    Write,
    Header,
}

impl ProcessErrorKind for TagsWriteErrorKind {}
type TagsWriteError = ProcessError<TagsWriteErrorKind>;

// Based on https://github.com/RustAudio/ogg/blob/0910d8d57645eccc1a1400731fefef376859c661/examples/repack.rs#L52
fn replace_header_comment(
    ogg_buffer: &Vec<u8>,
    comment_header: &lhr::CommentHeader,
) -> Result<Vec<u8>, TagsWriteError> {
    let mut out_buffer = io::Cursor::new(Vec::<u8>::new());
    let mut in_buffer = io::Cursor::new(ogg_buffer);

    let mut reader = ogg::PacketReader::new(&mut in_buffer);
    let mut writer = ogg::PacketWriter::new(&mut out_buffer);

    let mut overwrote_header = false;

    loop {
        if let Some(mut packet) = reader.read_packet().map_err(|e| TagsWriteError {
            kind: TagsWriteErrorKind::Read,
            error: e.into(),
        })? {
            if !overwrote_header {
                if let Ok(_) = lhr::read_header_comment(&packet.data) {
                    packet.data = make_header_comment(comment_header).ok_or(TagsWriteError {
                        kind: TagsWriteErrorKind::Header,
                        error: "invalid header comment data".into(),
                    })?;
                    overwrote_header = true;
                }
            }

            let packet_inf = if packet.last_in_stream() {
                ogg::PacketWriteEndInfo::EndStream
            } else if packet.last_in_page() {
                ogg::PacketWriteEndInfo::EndPage
            } else {
                ogg::PacketWriteEndInfo::NormalPacket
            };

            let packet_serial = packet.stream_serial();
            let packet_absgp = packet.absgp_page();

            writer
                .write_packet(packet.data, packet_serial, packet_inf, packet_absgp)
                .map_err(|e| TagsWriteError {
                    kind: TagsWriteErrorKind::Write,
                    error: e.into(),
                })?;
        } else {
            break;
        }
    }

    Ok(out_buffer.into_inner())
}
