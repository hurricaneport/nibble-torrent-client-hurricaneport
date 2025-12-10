use serde::{Deserialize, Serialize};
use crate::error::Error;
use url::Url;
use log::info;
use crate::networking;

#[derive(Serialize, Deserialize)]
#[derive(Clone)]
pub struct Torrent {
    pub torrent_id: String,
    pub tracker_url: Url,
    pub file_size: u64,
    pub piece_size: u64,
    pub pieces: Vec<String>,
    pub file_name: std::path::PathBuf,
}

#[derive(Serialize, Deserialize)]
pub struct TrackerResponse {
    pub interval: u64,
    pub peers: Vec<[String; 2]>,
}

#[derive(Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub enum TrackerMessage {
    UpdatedPeerList(Vec<[String; 2]>),
}

impl Torrent {
    /// Returns a Torrent object from a file with nibble torrent format JSON
    ///
    /// # Examples
    /// ```
    /// let torrent = Torrent::from_file("my_torrent.ntorrent");
    /// println!("Read torrent with ID: {}", torrent.torrent_id);
    ///```
    pub fn from_file(path: &std::path::PathBuf) -> std::result::Result<Torrent, Error> {
        let file_contents = std::fs::read_to_string(path)?;
         
        let torrent: Torrent = serde_json::from_str(&file_contents)?;

        Ok(torrent)

    }

    /// Sends a request to the tracker to get Peers and the refresh interval. Once called, this function should be called again after 'interval' seconds to get updated peer information.
    /// Also keeps the client registered with the tracker.
    /// # Examples
    /// ```
    /// let tracker_response = torrent.get_tracker_response("ECEN426-porter02", "10.0.0.8", 6881);
    /// println!("Tracker returned {} peers", tracker_response.peers.len());
    /// ```
    pub async fn get_tracker_response(&self, peer_id: &str, peer_ip: &str, peer_port: u16) -> std::result::Result<TrackerResponse, Error> {
        let url_with_params = self.tracker_url.join(&format!("?peer_id={}&ip={}&port={}&torrent_id={}",
            peer_id,
            peer_ip,
            peer_port,
            self.torrent_id
        ))?;

        let response_body = networking::send_http_request(crate::networking::Method::Get, url_with_params).await?;

        info!("Received tracker response: {}", &response_body);

        if let Ok(tracker_response) = serde_json::from_str::<TrackerResponse>(&response_body) {
            info!("Received tracker response with {} peers", tracker_response.peers.len());
            Ok(tracker_response)
        } else if let Ok(error_response)= serde_json::from_str::<ErrorResponse>(&response_body) {
            Err(Error::TrackerResponseError(error_response.error))
        } else {
            Err(Error::TrackerResponseError("Could not parse tracker response".to_string()))
        }
    }
}
