// SPDX-License-Identifier: GPL-3.0-or-later
//! Mock apcupsd NIS server for developing apctui without hardware.
//!
//!   cargo run --example mock_nis -- 3551 rack-main
//!   cargo run --example mock_nis -- 3552 rack-aux onbatt
//!
//! Serves a `status` response with drifting load, and (with the `onbatt`
//! flag) cycles through ONLINE -> ONBATT -> LOWBATT to exercise the UI.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64()
}

fn write_frame(s: &mut TcpStream, payload: &[u8]) -> std::io::Result<()> {
    s.write_all(&(payload.len() as u16).to_be_bytes())?;
    s.write_all(payload)
}

fn line(s: &mut TcpStream, key: &str, value: String) -> std::io::Result<()> {
    write_frame(s, format!("{key:<9}: {value}").as_bytes())
}

fn serve(mut conn: TcpStream, name: &str, cycle: bool) -> std::io::Result<()> {
    // read command frame
    let mut len = [0u8; 2];
    conn.read_exact(&mut len)?;
    let mut cmd = vec![0u8; u16::from_be_bytes(len) as usize];
    conn.read_exact(&mut cmd)?;
    if cmd != b"status" {
        write_frame(&mut conn, b"ERROR  : unsupported command")?;
        return write_frame(&mut conn, b"");
    }

    let t = now_secs();
    let load = 42.0 + 18.0 * (t / 23.0).sin() + 5.0 * (t / 3.7).sin();
    let phase = ((t / 45.0) as u64) % 4;
    let (status, bcharge, timeleft, linev) = if cycle {
        match phase {
            0 => ("ONLINE", 100.0, 28.0, 121.4),
            1 => ("ONBATT", 80.0 - (t % 45.0), 14.0 - (t % 45.0) / 4.0, 0.0),
            2 => ("ONBATT LOWBATT", 12.0, 2.5, 0.0),
            _ => ("ONLINE", 65.0 + (t % 45.0) / 2.0, 18.0, 120.9),
        }
    } else {
        ("ONLINE", 94.0 + (t / 60.0).sin(), 22.0 + 2.0 * (t / 40.0).sin(), 121.5)
    };

    line(&mut conn, "APC", "001,036,0857".into())?;
    line(&mut conn, "HOSTNAME", "mock".into())?;
    line(&mut conn, "VERSION", "3.14.14 (mock)".into())?;
    line(&mut conn, "UPSNAME", name.into())?;
    line(&mut conn, "MODEL", "Smart-UPS 1500 (mock)".into())?;
    line(&mut conn, "STATUS", status.into())?;
    line(&mut conn, "LINEV", format!("{linev:.1} Volts"))?;
    line(&mut conn, "LOADPCT", format!("{load:.1} Percent"))?;
    line(&mut conn, "BCHARGE", format!("{:.1} Percent", bcharge.clamp(0.0, 100.0)))?;
    line(&mut conn, "TIMELEFT", format!("{:.1} Minutes", timeleft.max(0.0)))?;
    line(&mut conn, "BATTV", format!("{:.1} Volts", 24.0 + bcharge / 25.0))?;
    line(&mut conn, "NOMPOWER", "900 Watts".into())?;
    line(&mut conn, "NUMXFERS", format!("{}", (t as u64 / 200) % 9))?;
    line(&mut conn, "SERIALNO", format!("MOCK{name}"))?;
    line(&mut conn, "END APC", "mock".into())?;
    write_frame(&mut conn, b"") // end of transmission
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().unwrap_or_else(|| "3551".into()).parse().expect("port");
    let name = args.next().unwrap_or_else(|| "mock-ups".into());
    let cycle = args.next().as_deref() == Some("onbatt");

    let listener = TcpListener::bind(("127.0.0.1", port))?;
    eprintln!("mock NIS `{name}` on 127.0.0.1:{port} (cycle={cycle})");
    for conn in listener.incoming().flatten() {
        let name = name.clone();
        std::thread::spawn(move || {
            let _ = serve(conn, &name, cycle);
        });
    }
    Ok(())
}
