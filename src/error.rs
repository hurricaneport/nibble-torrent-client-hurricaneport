use std::fmt;
#[derive(Debug)]
pub enum Error {
    IOError(std::io::Error),
    JSONError(serde_json::Error),
    HTTPParseError(httparse::Error),
    URLParseError(url::ParseError),
    TrackerResponseError(String),

}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::IOError(err) => write!(f, "IO Error: {}", err),
            Error::JSONError(err) => write!(f, "JSON Parse Error: {}", err),
            Error::HTTPParseError(err) => write!(f, "HTTP Parse Error: {}", err),
            Error::URLParseError(err) => write!(f, "URL Parse Error: {}", err),
            Error::TrackerResponseError(err) => write!(f, "Returned Error from the Server: {}", err),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from (err: std::io::Error) -> Self {
        Error::IOError(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from (err: serde_json::Error) -> Self {
        Error::JSONError(err)
    }
}

impl From<httparse::Error> for Error {
    fn from (err: httparse::Error) -> Self {
        Error::HTTPParseError(err)
    }
}

impl From<url::ParseError> for Error {
    fn from (err: url::ParseError) -> Self {
        Error::URLParseError(err)
    }
}
