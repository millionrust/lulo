//! Bounded niri IPC transport and request framing.

use super::*;

pub(super) async fn send(
    sender: &Sender<domain::Event>,
    event: domain::Event,
) -> Result<(), Error> {
    sender.send(event).await.map_err(|_| Error::ConsumerClosed)
}

#[cfg(unix)]
pub(super) type IpcStream = BufReader<Async<UnixStream>>;

#[cfg(unix)]
pub(super) fn connect(path: &Path) -> Result<IpcStream, Error> {
    let stream = UnixStream::connect(path)?;
    stream.set_nonblocking(true)?;
    Ok(BufReader::new(Async::new(stream)?))
}

/// niri runs only on Linux; elsewhere (System Settings on Windows) there
/// is no compositor socket to reach, and every request reports that.
#[cfg(not(unix))]
pub(super) type IpcStream = BufReader<Async<std::net::TcpStream>>;

#[cfg(not(unix))]
pub(super) fn connect(path: &Path) -> Result<IpcStream, Error> {
    let _ = path;
    Err(io::Error::new(io::ErrorKind::Unsupported, "niri runs only on Linux").into())
}

pub(super) async fn request_once(path: &Path, request: &Request) -> Result<Response, Error> {
    request_reply_once(path, request)
        .await?
        .map_err(Error::Protocol)
}

pub(super) async fn request_reply_once(path: &Path, request: &Request) -> Result<Reply, Error> {
    let mut stream = connect(path)?;
    write_request(&mut stream, request).await?;
    let line = read_line(&mut stream).await?;
    Ok(serde_json::from_str(&line)?)
}

pub(super) async fn write_request(stream: &mut IpcStream, request: &Request) -> Result<(), Error> {
    let mut bytes = serde_json::to_vec(request)?;
    bytes.push(b'\n');
    stream.get_mut().write_all(&bytes).await?;
    stream.get_mut().flush().await?;
    Ok(())
}

pub(super) async fn read_reply(stream: &mut IpcStream) -> Result<Response, Error> {
    let line = read_line(stream).await?;
    let reply: Reply = serde_json::from_str(&line)?;
    reply.map_err(Error::Protocol)
}

pub(super) async fn read_line(stream: &mut IpcStream) -> Result<String, Error> {
    let mut line = String::new();
    let count = stream.read_line(&mut line).await?;
    if count == 0 {
        return Err(Error::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "niri IPC socket closed",
        )));
    }
    while matches!(line.as_bytes().last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    Ok(line)
}
