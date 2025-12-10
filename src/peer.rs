use std::sync::Arc;

use log::{debug, error, info, warn};
use tokio::{net::TcpStream, sync::{Semaphore, mpsc}, task::JoinSet};

use crate::{error::Error, torrent::TrackerMessage};

enum PMControl {
    Stop
}

enum PollError {
    DropPeer(String),
    Other(String),
}
pub struct PeerState {
    id: String,
    addr: String,
    timeout_counter: u8,
    limit: Arc<Semaphore>,
}

pub struct PeerManagerState {
    pub peers: Vec<Arc<PeerState>>,
    pub needed_chunks: std::collections::HashSet<u64>,
    pub chunk_hashes: Vec<[u8; 20]>,
    pub output_file_folder: std::path::PathBuf,
    pub torrent_id: String,
    pub filter: Vec<String>,
    pub output_file: std::path::PathBuf,
}

impl PeerState{
    fn new(id: String, addr: String) -> Arc<Self> {
        Arc::new(Self {
            id,
            addr,
            timeout_counter: 0,
            limit: Arc::new(Semaphore::new(3)),
        })
    }

    fn record_timeout(&mut self) -> bool {
        self.timeout_counter += 1;
        self.timeout_counter >= 3
    }
}

pub async fn peer_manager_task(mut rx: mpsc::Receiver<TrackerMessage>, mut state: PeerManagerState, own_id: &str) -> Result<(), Error> {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));

    loop {
        tokio::select! {
            Some(message) = rx.recv() => {
                if let TrackerMessage::UpdatedPeerList(list) = message {
                    let filtered_list: Vec<[String; 2]> = list.into_iter()
                        .filter(|[id, _]| id != own_id && state.filter.is_empty() || state.filter.contains(id))
                        .collect();
                    state.peers = filtered_list.into_iter()
                        .map(|[id, ip]| {PeerState::new(ip, id)})
                        .collect();
                }
            },

            _ = interval.tick() => {
                match poll_all_peers(&mut state).await {
                    Ok(_) => {},
                    Err(e) => {
                        info!("Finished with all chunks, exiting peer manager task.");
                        return Ok(());
                    }
                }

                for i in 0..state.chunk_hashes.len() {
                    let path = format!("chunks/{}", i);
                    let dest_file = state.output_file_folder.join(path);

                    if !state.needed_chunks.contains(&(i as u64)) {
                        continue;
                    }

                    if tokio::fs::try_exists(dest_file).await? {
                        state.needed_chunks.remove(&(i as u64));
                    }
                }

                if state.needed_chunks.is_empty() {
                    info!("All chunks downloaded.");
                    break;
                }
            },
        }
    }

    compile_final_file(&state.output_file_folder, &state.output_file, state.chunk_hashes.len()).await?;

    Ok(())
}

async fn poll_all_peers(state: &mut PeerManagerState) -> Result<(), PMControl> {
    let mut set = JoinSet::new();

    let peers_snapshot = state.peers.clone();

    for peerinfo in peers_snapshot.iter() {
        let needed_chunks_clone = state.needed_chunks.clone();
        let output_file_clone = state.output_file_folder.clone();
        let peerinfo_clone = peerinfo.clone();
        let torrent_id = state.torrent_id.clone();
        let chunk_hashes = state.chunk_hashes.iter().cloned().collect();
        set.spawn( async move {
            poll_peer(&peerinfo_clone, needed_chunks_clone, output_file_clone, &torrent_id, chunk_hashes).await   
        });
    }

    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok(Some(PMControl::Stop))) => {
                return Err(PMControl::Stop);
            }
            Ok(Ok(None)) => {} // normal success
            Ok(Err(PollError::DropPeer(id))) => {
                warn!("Dropping peer {}", id);
                state.peers.retain(|p| p.id != id);
            }
            Ok(Err(PollError::Other(e))) => {
                warn!("Non-fatal error polling peer: {}", e);
            }
            Ok(Err(_)) => {
                warn!("Non-fatal error polling peer");
            } // non-fatal peer issues
            Err(_) => {} // task panicked — ignore or log
        }
    }

    Ok(())
}

async fn poll_peer(peer: &Arc<PeerState>, needed_chunks: std::collections::HashSet<u64>, output_file: std::path::PathBuf, torrent_id: &str, chunk_hashes: Vec<[u8; 20]>) -> Result<Option<PMControl>, PollError> {
    info!("Polling peer {} at {}", peer.id, peer.addr);
    let mut socket = TcpStream::connect(&peer.addr).await.map_err(|e| PollError::Other(format!("Error connecting to peer {}: {}", peer.id, e)))?;
    let available = get_peer_chunks(peer, torrent_id, &mut socket).await?;

    debug!("Peer {} has chunks: {:?}", peer.id, available.iter().enumerate().filter(|(_, has_chunk)| **has_chunk).map(|(idx, _)| idx).collect::<Vec<usize>>());

    let to_download: Vec<usize> = available.iter().enumerate()
        .filter(|(idx, has_chunk)| **has_chunk && needed_chunks.contains(&(*idx as u64)))
        .map(|(idx, _)| idx)
        .collect();

    if to_download.is_empty() {
        return Ok(None);
    }

    download_from_peer(peer, to_download, &output_file, &chunk_hashes, &mut socket).await?;
    
    Ok(None)
}

async fn get_peer_chunks(peer: &Arc<PeerState>, torrent_id: &str, socket: &mut TcpStream) -> Result<Vec<bool>, PollError> {
    debug!("get_peer_chunks({}, {})", peer.id, torrent_id);
    let method: u8 = 0x01;
    let data = hex::decode(torrent_id).unwrap().to_vec();
    let (response_method, response_data) = crate::networking::make_torrent_request(&peer.addr, method, data, socket).await.map_err(|e| PollError::Other(format!("Error making torrent request to peer {}: {}", peer.id, e)))?;
    debug!("Received response from peer {}: method 0x{:02x}, data len {}", peer.id, response_method, response_data.len());

    match response_method {
        0x02 => {}
        0x05 => {
            return Err(PollError::Other(format!("Peer {} returned error for chunk availability request: {}", peer.id, String::from_utf8_lossy(&response_data))));
        }
        _ => {
            return Err(PollError::Other(format!("Unexpected response method from peer {}: expected 0x02 or 0x05, got 0x{:02x}", peer.id, response_method)));
        }
    }
    let mut chunks: Vec<bool> = Vec::new();
    for i in 0..response_data.len() {
        let byte = response_data[i];
        for bit in 0..8 {
            let has_chunk = (byte & (1 << (7 - bit))) != 0;
            chunks.push(has_chunk);
        }
    }

    Ok(chunks)
}

async fn download_from_peer(peer: &Arc<PeerState>, chunk_indices: Vec<usize>, output_file: &std::path::PathBuf, chunk_hashes: &Vec<[u8; 20]>, socket: &mut TcpStream) -> Result<(), PollError> {
    for &chunk_idx in chunk_indices.iter() {
        if tokio::fs::try_exists(output_file.join(format!("chunks/{}", chunk_idx))).await.map_err(|e| PollError::Other(format!("Error checking existence of chunk {}: {}", chunk_idx, e)))? {
            debug!("Chunk {} already exists locally, skipping download from peer {}", chunk_idx, peer.id);
            continue;
        }
        let method = 0x03;
        let data = (chunk_idx as u64).to_be_bytes().to_vec();
        let (response_method, response_data) = crate::networking::make_torrent_request(&peer.addr, method, data, socket).await.map_err(|e| PollError::Other(format!("Error making torrent request to peer {}: {}", peer.id, e)))?;
        match response_method {
            0x04 => {}
            0x05 => {
                return Err(PollError::Other(format!("Peer {} returned error for chunk {} request: {}", peer.id, chunk_idx, String::from_utf8_lossy(&response_data))));
            }
            _ => {
                return Err(PollError::Other(format!("Unexpected response method from peer {}: expected 0x04 or 0x05, got 0x{:02x}", peer.id, response_method)));
            }
        }

        let expected_hash = chunk_hashes.get(chunk_idx).ok_or_else(|| PollError::Other(format!("Invalid chunk index {} for peer {}", chunk_idx, peer.id)))?;
        let received_hash = calculate_hash(&response_data);
        if &received_hash != expected_hash {
            return Err(PollError::Other(format!("Hash mismatch for chunk {} from peer {}: {:?} vs {:?}", chunk_idx, peer.id, expected_hash, received_hash)));
        }

        let chunk_path = output_file.join(format!("chunks/{}", chunk_idx));
        tokio::fs::create_dir_all(chunk_path.parent().unwrap()).await.map_err(|e| PollError::Other(format!("Error creating directories for chunk {}: {}", chunk_idx, e)))?;
        tokio::fs::write(&chunk_path, &response_data).await.map_err(|e| PollError::Other(format!("Error writing chunk {} to file: {}", chunk_idx, e)))?;


    }

    Ok(())
}

/// Calculates SHA-1 hash of the given data
fn calculate_hash(data: &Vec<u8>) -> [u8; 20] {
    use sha1::{Sha1, Digest};
    let mut hasher = Sha1::new();
    hasher.update(data);
    let result = hasher.finalize();
    result.as_slice().try_into().unwrap()
}

async fn compile_final_file(output_file_folder: &std::path::PathBuf, output_file: &std::path::PathBuf, total_chunks: usize) -> Result<(), Error> {
    let mut final_file = tokio::fs::File::create(output_file).await?;

    for i in 0..total_chunks {
        let chunk_path = output_file_folder.join(format!("chunks/{}", i));
        let chunk_data = tokio::fs::read(&chunk_path).await?;
        tokio::io::AsyncWriteExt::write_all(&mut final_file, &chunk_data).await?;
    }

    info!("Successfully compiled final file at {:?}", output_file);
    Ok(())
}