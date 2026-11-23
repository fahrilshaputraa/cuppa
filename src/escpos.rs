// SPDX-License-Identifier: AGPL-3.0-or-later
//! Minimal ESC/POS builder + LAN sender (raw TCP, usually port 9100). Kept dependency-free so other
//! transports (CloudPRNT, local agent) can reuse `receipt_bytes`.

use crate::money::rupiah;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tokio::{io::AsyncWriteExt, net::TcpStream};

pub struct ReceiptLine {
    pub qty: i64,
    pub name: String,
    pub amount: i64,
}

pub struct Receipt<'a> {
    pub outlet: &'a str,
    pub number: i64,
    pub where_label: &'a str,
    pub when: &'a str,
    pub lines: &'a [ReceiptLine],
    pub subtotal: i64,
    pub service: i64,
    pub tax: i64,
    pub total: i64,
    pub discount: i64,
    pub rounding: i64,
    /// (method label, amount applied, cash tendered)
    pub payments: &'a [(String, i64, Option<i64>)],
    pub footer: &'a str,
    pub cols: usize,
    pub open_drawer: bool,
}

fn ascii(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii() && !c.is_control() { c } else { '?' })
        .collect()
}

fn two_col(left: &str, right: &str, cols: usize) -> String {
    let right = ascii(right);
    let room = cols.saturating_sub(right.len() + 1);
    let left: String = ascii(left).chars().take(room).collect();
    format!("{left}{}{right}\n", " ".repeat(cols - left.len() - right.len()))
}

pub fn receipt_bytes(r: &Receipt) -> Vec<u8> {
    let mut b: Vec<u8> = vec![0x1b, b'@']; // init
    let text = |b: &mut Vec<u8>, s: &str| b.extend_from_slice(ascii(s).as_bytes());
    b.extend_from_slice(&[0x1b, b'a', 1, 0x1b, b'E', 1]); // center, bold
    text(&mut b, &format!("{}\n", r.outlet));
    b.extend_from_slice(&[0x1b, b'E', 0]);
    text(&mut b, &format!("#{} - {}\n{}\n", r.number, r.where_label, r.when));
    b.extend_from_slice(&[0x1b, b'a', 0]); // left
    text(&mut b, &format!("{}\n", "-".repeat(r.cols)));
    for l in r.lines {
        text(
            &mut b,
            &two_col(&format!("{}x {}", l.qty, l.name), &rupiah(l.amount), r.cols),
        );
    }
    text(&mut b, &format!("{}\n", "-".repeat(r.cols)));
    text(&mut b, &two_col("Subtotal", &rupiah(r.subtotal), r.cols));
    if r.discount > 0 {
        text(&mut b, &two_col("Diskon", &format!("-{}", rupiah(r.discount)), r.cols));
    }
    if r.service > 0 {
        text(&mut b, &two_col("Service", &rupiah(r.service), r.cols));
    }
    if r.tax > 0 {
        text(&mut b, &two_col("Pajak", &rupiah(r.tax), r.cols));
    }
    if r.rounding != 0 {
        text(&mut b, &two_col("Pembulatan", &rupiah(r.rounding), r.cols));
    }
    b.extend_from_slice(&[0x1b, b'E', 1]);
    text(&mut b, &two_col("TOTAL", &rupiah(r.total), r.cols));
    b.extend_from_slice(&[0x1b, b'E', 0]);
    for (label, amount, tendered) in r.payments {
        text(
            &mut b,
            &two_col(
                &format!("Bayar ({label})"),
                &rupiah(tendered.unwrap_or(*amount)),
                r.cols,
            ),
        );
        if let Some(t) = tendered.filter(|t| t > amount) {
            text(&mut b, &two_col("Kembali", &rupiah(t - amount), r.cols));
        }
    }
    b.extend_from_slice(&[0x1b, b'a', 1]);
    text(&mut b, &format!("\n{}\n\n\n\n", r.footer));
    b.extend_from_slice(&[0x1d, b'V', 66, 0]); // feed + partial cut
    if r.open_drawer {
        b.extend_from_slice(&drawer_pulse());
    }
    b
}

/// Kitchen/bar ticket: big, no prices.
pub fn ticket_bytes(number: i64, where_label: &str, when: &str, lines: &[String], note: Option<&str>) -> Vec<u8> {
    let mut b: Vec<u8> = vec![0x1b, b'@', 0x1b, b'a', 1, 0x1d, b'!', 0x11]; // center, double size
    b.extend_from_slice(ascii(&format!("#{number}\n{where_label}\n")).as_bytes());
    b.extend_from_slice(&[0x1d, b'!', 0x00, 0x1b, b'a', 0]);
    b.extend_from_slice(ascii(&format!("{when}\n--------------------------------\n")).as_bytes());
    for l in lines {
        b.extend_from_slice(ascii(&format!("{l}\n")).as_bytes());
    }
    if let Some(n) = note {
        b.extend_from_slice(ascii(&format!("\nCatatan: {n}\n")).as_bytes());
    }
    b.extend_from_slice(b"\n\n\n\n");
    b.extend_from_slice(&[0x1d, b'V', 66, 0]);
    b
}

/// ESC p m t1 t2: pulse pin 2.
pub fn drawer_pulse() -> [u8; 5] {
    [0x1b, b'p', 0, 25, 250]
}

/// Only LAN printers: the host must be a private/loopback/link-local IP literal, so a compromised
/// owner session cannot turn the printer feature into a scanner for the public internet.
pub fn parse_printer(s: &str) -> Result<SocketAddr, String> {
    let addr: SocketAddr = s
        .trim()
        .parse()
        .map_err(|_| "Format printer: 192.168.1.50:9100".to_string())?;
    let ok = match addr.ip() {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_loopback(),
    };
    if ok {
        Ok(addr)
    } else {
        Err("Printer harus berada di jaringan lokal (IP privat).".into())
    }
}

pub async fn send(addr: SocketAddr, data: &[u8]) -> anyhow::Result<()> {
    let mut s = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(addr)).await??;
    tokio::time::timeout(Duration::from_secs(5), async {
        s.write_all(data).await?;
        s.flush().await
    })
    .await??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printer_must_be_lan() {
        assert!(parse_printer("192.168.1.50:9100").is_ok());
        assert!(parse_printer("10.0.0.2:9100").is_ok());
        assert!(parse_printer("8.8.8.8:9100").is_err());
        assert!(parse_printer("example.com:9100").is_err());
    }

    #[test]
    fn receipt_has_init_cut_and_total() {
        let lines = [ReceiptLine {
            qty: 2,
            name: "Americano".into(),
            amount: 36_000,
        }];
        let r = Receipt {
            outlet: "Kopi",
            number: 7,
            where_label: "Meja 1",
            when: "10:00",
            lines: &lines,
            subtotal: 36_000,
            service: 0,
            tax: 3_600,
            total: 39_600,
            discount: 0,
            rounding: 0,
            payments: &[("Tunai".to_string(), 39_600, Some(50_000))],
            footer: "Terima kasih",
            cols: 32,
            open_drawer: true,
        };
        let b = receipt_bytes(&r);
        let s = String::from_utf8_lossy(&b);
        assert!(b.starts_with(&[0x1b, b'@']) && s.contains("Rp39.600") && s.contains("Kembali"));
        assert!(b.windows(4).any(|w| w == [0x1d, b'V', 66, 0]) && b.ends_with(&drawer_pulse()));
    }
}
