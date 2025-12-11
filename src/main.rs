use clap::Parser;
use nibble_torrent_client_hurricaneport::torrent::{Torrent, TrackerMessage};
use log::{debug, info, warn, error, LevelFilter};
use simple_logger::SimpleLogger;
use nibble_torrent_client_hurricaneport::error::Error;
use std::{collections::HashSet, hash::Hash, net::UdpSocket, path::Path, sync::Arc};
use tokio::{sync::{Semaphore, mpsc}, task::JoinSet};
use nibble_torrent_client_hurricaneport::{peer,server};


#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Your NetID
    netid: String,

    /// The torrent file for the file you want to download.
    torrent_file: std::path::PathBuf,

    /// The port to receive peer connections from.
    #[arg(short, long, default_value_t = 8088)]
    port: u16,

    /// The folder to download to and seed from.
    #[arg(short, long)]
    dest: std::path::PathBuf,

    /// The NetID of the peer you will only download from.
    #[arg(short, long, value_name = "NETID")]
    filter: Option<String>,

    /// Turn on debugging messages.
    #[arg(short, long, default_value_t = false)]
    verbose: bool,
}




#[tokio::main]
async fn main() {
    let args = Args::parse(); 

    match SimpleLogger::new().with_level(if args.verbose {LevelFilter::Debug} else {LevelFilter::Warn}).with_colors(true).init() {
        Ok(_) => {}
        Err(e) => {
            error!("Error while initializing logger: {}", e);
            std::process::exit(1);
        }
    }

    println!("Ran with Port: {}", args.port);
    
    let torrent = match Torrent::from_file(&args.torrent_file) {
        Ok(torrent) => torrent,
        Err(e) => {
            match e {
                Error::IOError(err) => error!("Could not find file {}: {}", &args.torrent_file.display(), err),
                Error::JSONError(err) => error!("File {} has incorrect ntorrent formatting: {}", &args.torrent_file.display(), err),
                _ => error!("Unexpected error while reading torrent file {}: {}", &args.torrent_file.display(), e),
            }
        std::process::exit(1);
        }
    };

    let peer_id = format!("-ECEN426-{}", args.netid);
    let torrent_tracker_host = format!("{}:{}", torrent.tracker_url.host_str().unwrap(), torrent.tracker_url.port_or_known_default().unwrap_or(8088));
    let peer_ip = match get_outbound_ip(&torrent_tracker_host) {
        Ok(ip) => ip,
        Err(e) => {
            error!("Could not determine outbound IP address: {}", e);
            std::process::exit(1);
        }
    };

    let mut needed_chunks = HashSet::new();

    for i in 0..torrent.pieces.len() {
        needed_chunks.insert(i as u64);
    }

    let mut chunk_hashes = Vec::new();
    for piece_hash in torrent.pieces.iter() {
        let hash = hex::decode(piece_hash).unwrap().as_slice().try_into().unwrap();
        chunk_hashes.push(hash);
    }

    let peer_manager_state = peer::PeerManagerState {
        peers: Vec::new(),
        needed_chunks: needed_chunks,
        chunk_hashes: chunk_hashes,
        output_file_folder: args.dest.clone(),
        torrent_id: torrent.torrent_id.clone(),
        filter: match args.filter {
            Some(netid) => {vec![netid]},
            None => Vec::new(),
        },
        output_file: args.dest.join(torrent.file_name.clone()),


    };

    let (tx_tracker, rx_tracker) = tokio::sync::mpsc::channel(32);
    let torrent_server = torrent.clone();
    let peer_id_clone = peer_id.clone();
    tokio::spawn( async move {
        if let Err(e) = tracker_task(tx_tracker, torrent, peer_id.clone(), peer_ip.clone(), args.port).await {
            error!("Error in tracker task: {}", e);
        }
    });

    tokio::spawn( async move {
        if let Err(e) = peer::peer_manager_task(rx_tracker, peer_manager_state, &format!("-ECEN426-{}",&args.netid)).await {
            error!("Error in peer manager task: {}", e);
        }
    });

    tokio::spawn( async move {
        if let Err(e) = server::server_task(args.port, args.dest, torrent_server, peer_id_clone).await {
            error!("Error in server task: {}", e);
        }
    });

    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl-c signal");
    info!("Received Ctrl-C, shutting down.");
}

async fn tracker_task(tx: mpsc::Sender<TrackerMessage>, torrent: Torrent, peer_id: String, peer_ip: String, peer_port: u16) -> Result<(), Error> {
    loop {
        let interval: u64;
        let tracker_response = torrent.get_tracker_response(&peer_id, &peer_ip, peer_port).await?;
            
        info!("Received tracker response with {} peers", tracker_response.peers.len());
        interval = tracker_response.interval;
        match tx.send(TrackerMessage::UpdatedPeerList(tracker_response.peers)).await {
            Ok(_) => {},
            Err(e) => {
                error!("Error sending updated peer list to main task: {}", e);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;  
    }
}

fn get_outbound_ip(dest: &str) -> std::io::Result<String> {
    debug!("Determining outbound IP address by connecting to {}", dest);

    // Create a UDP socket and "connect" to the destination
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect(dest)?;

    // Get the local address the OS selected
    let local_addr = socket.local_addr()?;
    Ok(local_addr.ip().to_string())
}
