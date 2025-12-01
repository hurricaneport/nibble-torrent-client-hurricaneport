use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Your NetID
    netid: String,

    /// The torrent file for the file you want to download.
    torrent_file: std::path::PathBuf,

    /// The port to receive peer connections from.
    #[arg(short, long)]
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

fn main() {
    let args = Args::parse(); 
    
   

}
