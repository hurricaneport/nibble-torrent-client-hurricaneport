use crate::error::Error;
use url::Url;
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use httparse::{Header, Response, Status};
use log::{debug, info};
pub enum Method {
    Get,
    Post,
    Put,
    Delete
}

impl Method {
    pub fn to_string(&self) -> String {
        match self {
            Method::Get => "GET".to_owned(),
            Method::Post => "POST".to_owned(),
            Method::Put => "PUT".to_owned(),
            Method::Delete => "DELETE".to_owned()
        }
    }
}

pub async fn send_http_request(method: Method, url: Url) -> Result<String, Error> {
    let mut path_and_query = url.path().to_string();

    if let Some(query) = url.query() {
        path_and_query.push('?');
        path_and_query.push_str(query);

    }

    let request_message = format!("{} {} HTTP/1.1\r\nHost: {}\r\n\r\n", method.to_string(), path_and_query, url.host_str().unwrap());
    let request_message_bytes = request_message.as_bytes();

    

    let addr = format!("{}:{}", url.host_str().unwrap(), url.port_or_known_default().unwrap_or(8088));

    info!("Connecting to address: {}", addr);
    let mut stream = TcpStream::connect(addr).await?;
    
    stream.write_all(request_message_bytes).await?;

    let mut response_buf: Vec<u8> = Vec::new();
    let mut temp = [0u8; 1024];
    let mut response: Response<'_, '_>;
    let mut header_array : [Header<'_>; 16];
  
    let header_end = loop {
      let n = stream.read(& mut temp).await?;

      if n == 0 {
        return Err(Error::IOError(std::io::Error::new(std::io::ErrorKind::Interrupted, "Stream closed before all data received")));
      }

      response_buf.extend_from_slice(&temp[..n]);

      header_array = [httparse::EMPTY_HEADER; 16];

      response = Response::new(&mut header_array);

      match response.parse(&response_buf)? {
        Status::Complete(n) => break n,
        Status::Partial => continue,
      }

    };

    let content_length = response.headers.iter().find(|h| h.name.eq_ignore_ascii_case("Content-Length")).map(|h| h.value);

    match content_length {
        Some(cl_bytes) => {
            let cl_str = std::str::from_utf8(cl_bytes).map_err(|e| Error::IOError(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Invalid Content-Length header: {}", e))))?;
            let cl: usize = cl_str.parse().map_err(|e| Error::IOError(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Could not parse Content-Length header: {}", e))))?;

            let total_len = header_end + cl;

            while response_buf.len() < total_len {
                let n = stream.read(& mut temp).await?;

                if n == 0 {
                    return Err(Error::IOError(std::io::Error::new(std::io::ErrorKind::Interrupted, "Stream closed before all data received")));
                }

                response_buf.extend_from_slice(&temp[..n]);
            }

            let body_bytes = &response_buf[header_end..total_len];
            let body_str = std::str::from_utf8(body_bytes).map_err(|e| Error::IOError(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Invalid body data: {}", e))))?;

            Ok(body_str.to_owned())
        },
        None => Err(Error::IOError(std::io::Error::new(std::io::ErrorKind::InvalidData, "No Content-Length header in response"))),
    }

}

pub async fn make_torrent_request(addr: &str, message_type: u8, data: Vec<u8>, socket: &mut TcpStream) -> Result<(u8, Vec<u8>), Error> {
    send_message(socket, message_type, &data).await?;
    debug!("Sent message type 0x{:02x} to peer at {}", message_type, addr);

    //Await response header
    let (response_type, response_data) = receive_message(socket).await?;
    debug!("Received response type 0x{:02x} from peer at {}", response_type, addr);

    Ok((response_type, response_data))
}

pub async fn receive_message(socket: &mut TcpStream) -> Result<(u8, Vec<u8>), Error> {
  let mut response_header = [0u8; 4];
  socket.read_exact(&mut response_header).await?;


  //Parse response header
  let response_version = response_header[0];
  let response_type = response_header[1];
  let response_len = u16::from_be_bytes([response_header[2], response_header[3]]) as usize;

  if response_version != 0x01 {
        return Err(Error::IOError(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid response version")));
    }

  //Read response data
  let mut response_data = vec![0u8; response_len];
  socket.read_exact(&mut response_data).await?;

  Ok((response_type, response_data))
}

pub async fn send_message(socket: &mut TcpStream, message_type: u8, data: &Vec<u8>) -> Result<(), Error> {
    if data.len() > u16::MAX as usize {
        return Err(Error::IOError(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Data length exceeds maximum allowed size")));
    }

    let len_bytes = (data.len() as u16).to_be_bytes();

    let mut header = [0u8; 4];
    header[0] = 0x01; // Version  
    header[1] = message_type;
    header[2] = len_bytes[0];
    header[3] = len_bytes[1];

    // Send header and data
    socket.write_all(&header).await?;
    socket.write_all(data).await?;

    Ok(())
}
