use url::Url;

pub struct Torrent {
    torrent_id: String,
    tracker_url: Url,
    file_size: u64,
    piece_size: u64,
    pieces: Vec<String>,
    file_name: std::path::PathBuf,
}
