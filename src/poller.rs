// SPDX-License-Identifier: GPL-3.0-or-later
//! One polling thread per UPS, pushing updates to the UI over a channel.

use crate::nis::{self, UpsStatus};
use std::sync::mpsc::Sender;
use std::time::Duration;

pub struct Update {
    pub idx: usize,
    pub result: Result<UpsStatus, String>,
}

pub fn spawn(idx: usize, addr: String, interval: Duration, tx: Sender<Update>) {
    std::thread::Builder::new()
        .name(format!("poller-{idx}"))
        .spawn(move || loop {
            let result = nis::fetch_status(&addr, Duration::from_secs(3))
                .map_err(|e| format!("{e:#}"));
            if tx.send(Update { idx, result }).is_err() {
                return; // UI gone, exit thread
            }
            std::thread::sleep(interval);
        })
        .expect("spawn poller thread");
}
