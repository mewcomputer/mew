//! Restricted CONNECT proxy for the isolated local AGY process.

use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

const ALLOWED: &[&str] = &[
    "www.google.com",
    "accounts.google.com",
    "oauth2.googleapis.com",
    "play.googleapis.com",
    "www.googleapis.com",
    "cloudcode-pa.googleapis.com",
    "daily-cloudcode-pa.googleapis.com",
    "generativelanguage.googleapis.com",
    "antigravity.google.com",
];
const MAX_HEADER: usize = 8192;

pub struct EgressProxy {
    port: u16,
    stop: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl EgressProxy {
    pub async fn start() -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        #[cfg(target_os = "macos")]
        let reserved_ipv6 = TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, port)).await?;
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let task = tokio::spawn(async move {
            #[cfg(target_os = "macos")]
            let _reserved_ipv6 = reserved_ipv6;
            let limit = Arc::new(Semaphore::new(64));
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    _ = stopped.cancelled() => break,
                    result = listener.accept() => {
                        let Ok((socket, _)) = result else { break };
                        let Ok(permit) = limit.clone().try_acquire_owned() else { continue };
                        connections.spawn(async move {
                            let _permit = permit;
                            let _ = handle(socket).await;
                        });
                    },
                    Some(_) = connections.join_next(), if !connections.is_empty() => {},
                }
            }
            connections.shutdown().await;
        });
        Ok(Self {
            port,
            stop,
            task: Some(task),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn shutdown(&mut self) -> io::Result<()> {
        self.stop.cancel();
        if let Some(task) = self.task.as_mut() {
            task.await.map_err(io::Error::other)?;
            self.task.take();
        }
        Ok(())
    }
}

impl Drop for EgressProxy {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

async fn handle(mut client: TcpStream) -> io::Result<()> {
    let (header, end) = tokio::time::timeout(Duration::from_secs(15), read_header(&mut client))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "proxy header timeout"))??;
    let first_line = header.split(|b| *b == b'\n').next().unwrap_or_default();
    let request = std::str::from_utf8(first_line)
        .unwrap_or("")
        .trim_end_matches('\r');
    let mut parts = request.split(' ');
    let method = parts.next();
    let target = parts.next();
    let version = parts.next();
    let host = target.and_then(|v| v.strip_suffix(":443"));
    let host = host.filter(|host| ALLOWED.contains(host));
    if method != Some("CONNECT")
        || !matches!(version, Some("HTTP/1.1" | "HTTP/1.0"))
        || parts.next().is_some()
        || host.is_none()
    {
        client
            .write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")
            .await?;
        return Ok(());
    }
    let host = host.expect("checked");
    let mut upstream = tokio::time::timeout(Duration::from_secs(15), async {
        let addrs = tokio::net::lookup_host((host, 443)).await?;
        for addr in addrs {
            if let SocketAddr::V4(addr) = addr {
                if public_ip(*addr.ip()) {
                    if let Ok(socket) = TcpStream::connect(addr).await {
                        return Ok(socket);
                    }
                }
            }
        }
        Err(io::Error::other("approved destination unavailable"))
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "proxy connection timeout"))??;
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    upstream.write_all(&header[end..]).await?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

async fn read_header(socket: &mut TcpStream) -> io::Result<(Vec<u8>, usize)> {
    let mut data = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        let count = socket.read(&mut chunk).await?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete proxy request",
            ));
        }
        data.extend_from_slice(&chunk[..count]);
        if let Some(pos) = data.windows(4).position(|v| v == b"\r\n\r\n") {
            if pos + 4 > MAX_HEADER {
                return Err(io::Error::other("proxy header too large"));
            }
            return Ok((data, pos + 4));
        }
        if data.len() > MAX_HEADER {
            return Err(io::Error::other("proxy header too large"));
        }
    }
}

fn public_ip(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || a == 0
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 198 && (b == 18 || b == 19))
        || (a == 192 && b == 0 && c == 0))
}

#[cfg(test)]
mod tests {
    use super::public_ip;
    use std::net::Ipv4Addr;

    #[test]
    fn rejects_local_and_reserved_addresses_from_dns() {
        for address in [
            "0.0.0.1",
            "10.0.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.31.1.1",
            "192.168.0.1",
            "100.64.0.1",
            "198.18.0.1",
            "192.0.2.1",
            "224.0.0.1",
            "240.0.0.1",
        ] {
            assert!(
                !public_ip(address.parse::<Ipv4Addr>().unwrap()),
                "{address}"
            );
        }
        assert!(public_ip("8.8.8.8".parse().unwrap()));
    }
}
