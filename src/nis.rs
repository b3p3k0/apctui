// SPDX-License-Identifier: GPL-3.0-or-later
//! Native client for apcupsd's Network Information Server (NIS) protocol.
//!
//! The protocol is trivial: a TCP stream of frames, each prefixed with a
//! 2-byte big-endian length. The client sends a command frame ("status" or
//! "events"); the server replies with one frame per line and terminates the
//! response with a zero-length frame.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const MAX_FRAME: usize = 8192;

fn write_frame(stream: &mut TcpStream, payload: &[u8]) -> Result<()> {
    let len = u16::try_from(payload.len()).context("frame too large")?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(payload)?;
    Ok(())
}

/// Read one frame. `Ok(None)` means the zero-length end-of-transmission frame.
fn read_frame(stream: &mut TcpStream) -> Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf)?;
    let len = u16::from_be_bytes(len_buf) as usize;
    if len == 0 {
        return Ok(None);
    }
    if len > MAX_FRAME {
        bail!("oversized NIS frame ({len} bytes)");
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(Some(buf))
}

fn connect(addr: &str, timeout: Duration) -> Result<TcpStream> {
    let sock_addr = addr
        .to_socket_addrs()
        .with_context(|| format!("cannot resolve {addr}"))?
        .next()
        .with_context(|| format!("no address for {addr}"))?;
    let stream = TcpStream::connect_timeout(&sock_addr, timeout)
        .with_context(|| format!("connect to {addr}"))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    Ok(stream)
}

/// Run one NIS command and collect the response lines.
pub fn command(addr: &str, cmd: &str, timeout: Duration) -> Result<Vec<String>> {
    let mut stream = connect(addr, timeout)?;
    write_frame(&mut stream, cmd.as_bytes())?;
    let mut lines = Vec::new();
    while let Some(frame) = read_frame(&mut stream)? {
        lines.push(String::from_utf8_lossy(&frame).into_owned());
    }
    Ok(lines)
}

/// A parsed `status` response. Keys are upper-case directive names
/// (e.g. "STATUS", "LINEV"); values are trimmed strings with units intact.
#[derive(Debug, Clone, Default)]
pub struct UpsStatus {
    pub fields: HashMap<String, String>,
}

impl UpsStatus {
    pub fn parse(lines: &[String]) -> Self {
        let mut fields = HashMap::new();
        for line in lines {
            if let Some((key, value)) = line.split_once(':') {
                fields.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
        UpsStatus { fields }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    /// Numeric fields are formatted as "<number> <unit>"; parse the number.
    pub fn num(&self, key: &str) -> Option<f64> {
        self.get(key)?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    }

    pub fn status_text(&self) -> &str {
        self.get("STATUS").unwrap_or("UNKNOWN")
    }

    /// Estimated output watts: NOMPOWER * LOADPCT / 100.
    pub fn watts(&self) -> Option<f64> {
        Some(self.num("NOMPOWER")? * self.num("LOADPCT")? / 100.0)
    }
}

pub fn fetch_status(addr: &str, timeout: Duration) -> Result<UpsStatus> {
    let lines = command(addr, "status", timeout)?;
    if lines.is_empty() {
        bail!("empty status response from {addr}");
    }
    Ok(UpsStatus::parse(&lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[&str] = &[
        "APC      : 001,036,0857",
        "DATE     : 2026-06-10 09:41:22 -0400",
        "HOSTNAME : minipc",
        "UPSNAME  : rack-main",
        "MODEL    : Smart-UPS 1500",
        "STATUS   : ONLINE",
        "LINEV    : 121.5 Volts",
        "LOADPCT  : 48.0 Percent",
        "BCHARGE  : 94.0 Percent",
        "TIMELEFT : 22.0 Minutes",
        "BATTV    : 27.3 Volts",
        "NOMPOWER : 900 Watts",
        "END APC  : 2026-06-10 09:41:22 -0400",
    ];

    fn sample() -> UpsStatus {
        UpsStatus::parse(&SAMPLE.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parses_fields() {
        let s = sample();
        assert_eq!(s.status_text(), "ONLINE");
        assert_eq!(s.get("MODEL"), Some("Smart-UPS 1500"));
        assert_eq!(s.num("LOADPCT"), Some(48.0));
        assert_eq!(s.num("TIMELEFT"), Some(22.0));
    }

    #[test]
    fn computes_watts() {
        assert_eq!(sample().watts(), Some(432.0));
    }

    #[test]
    fn frame_roundtrip_over_loopback() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            // read command frame
            let mut len = [0u8; 2];
            conn.read_exact(&mut len).unwrap();
            let mut cmd = vec![0u8; u16::from_be_bytes(len) as usize];
            conn.read_exact(&mut cmd).unwrap();
            assert_eq!(cmd, b"status");
            for line in SAMPLE {
                let b = line.as_bytes();
                conn.write_all(&(b.len() as u16).to_be_bytes()).unwrap();
                conn.write_all(b).unwrap();
            }
            conn.write_all(&0u16.to_be_bytes()).unwrap(); // EOT
        });
        let status = fetch_status(&addr.to_string(), Duration::from_secs(2)).unwrap();
        assert_eq!(status.status_text(), "ONLINE");
        server.join().unwrap();
    }
}
