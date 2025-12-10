use std::sync::Arc;
use std::vec;

use log::{debug, info, warn};
use tokio::net::{TcpListener, TcpStream};
use tokio::fs::try_exists;

use crate::{error::Error, torrent::Torrent};


pub async fn server_task(port: u16, dest_folder: std::path::PathBuf, torrent: Torrent, peer_id: String) -> Result<(), Error> {
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind((addr.as_str())).await?;
    info!("Server listening on {}", addr);
    let torrent = Arc::new(torrent);
    loop {
        let (mut socket, peer_addr) = listener.accept().await?;
        let dest_folder_clone = dest_folder.clone();
        let torrent_clone = Arc::clone(&torrent);
        
        tokio::spawn( async move {
            info!("Accepted connection from {}", peer_addr);
            handle_peer_connection(socket, dest_folder_clone, torrent_clone).await
                .unwrap_or_else(|e| warn!("Error handling connection from {}: {}", peer_addr, e));
        });
    }
}

pub async fn handle_peer_connection(mut socket: TcpStream, dest_folder: std::path::PathBuf, torrent: Arc<Torrent>) -> Result<(), Error> {
    loop {
        let (message_type, payload) = crate::networking::receive_message(&mut socket).await?;
        match message_type {
            0x01 => {
                info!("Received Hello request from peer");
                match handle_hello_request(&mut socket, &dest_folder, Arc::clone(&torrent)).await {
                    Ok(_) => {},
                    Err(e) => {
                        warn!("Error handling Hello request from peer: {}", e);
                    }
                }
            },
            0x03 => {
                info!("Received chunk request from peer: {:?}", payload);
                match handle_chunk_request(&mut socket, &dest_folder, &torrent, payload).await {
                    Ok(_) => {},
                    Err(e) => {
                        warn!("Error handling chunk request from peer: {}", e);
                    }
                }
            },
            0x05 => {
                info!("Received Error message from peer: {:?}", payload);
            },
            _ => {
                warn!("Received unknown message type 0x{:02x} from peer", message_type);
            }
        }
    }
    Ok(())
}

pub async fn handle_hello_request(socket: &mut TcpStream, dest_folder: &std::path::PathBuf, torrent: Arc<Torrent>) -> Result<(), Error> {
    // Send back a Hello response
    let mut available_chunks: Vec<u8> = Vec::new(); // TODO: Fill this with actual chunk availability
    
    for (idx, _chunk_hash) in torrent.pieces.iter().enumerate() {
        let path = format!("chunks/{}", idx);
        let dest_file = dest_folder.join(path);

        if try_exists(dest_file).await? {available_chunks.push(1);} else {available_chunks.push(0);}
    }

    let mut response_buf = vec![0u8; (available_chunks.len() + 7) / 8];
    for (i, &has_chunk) in available_chunks.iter().enumerate() {
        if has_chunk != 0 {
            let byte_index = i / 8;
            let bit_index = 7 - (i % 8);
            response_buf[byte_index] |= 1 << bit_index;
        }
    }

    debug!("Sending Hello response with {} bytes of chunk availability", response_buf.len());
    crate::networking::send_message(socket, 0x02, &response_buf).await?;
    Ok(())
}

pub async fn handle_chunk_request(socket: &mut TcpStream, dest_folder: &std::path::PathBuf, torrent: &Torrent, message_payload: Vec<u8>) -> Result<(), Error> {
    if message_payload.is_empty() {
        return Err(Error::IOError(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid chunk request payload length")));
    }

    let mut buf = [0u8; 8];

    let bytes = &message_payload[..message_payload.len().min(8)];
    buf[8 - bytes.len()..].copy_from_slice(bytes); // left-pad with zeros (big-endian)

    let chunk_idx = u64::from_be_bytes(buf) as usize;

    let path = format!("chunks/{}", chunk_idx);
    let dest_file = dest_folder.join(path);

    if !tokio::fs::try_exists(dest_file.clone()).await? {
        crate::networking::send_message(socket, 0x05, &b"Chunk not found".to_vec()).await?;
        return Err(Error::IOError(std::io::Error::new(std::io::ErrorKind::NotFound, format!("Requested chunk {} not found", chunk_idx))));
    }

    let chunk_data = tokio::fs::read(dest_file).await?;
    crate::networking::send_message(socket, 0x04, &chunk_data).await?;
    debug!("Sent chunk {} with {} bytes of data", chunk_idx, chunk_data.len());
    Ok(())
}