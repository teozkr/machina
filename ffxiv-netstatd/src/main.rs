use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    io::Read,
    os::{linux::fs::MetadataExt, unix::prelude::FileTypeExt},
    path::{Path, PathBuf},
};

use axum::{Json, Router};
use serde::Serialize;

fn find_ffxiv_proc_path() -> Option<PathBuf> {
    let proc = Path::new("/proc");
    for process in proc.read_dir().unwrap() {
        let process = process.unwrap();
        if !process.file_type().unwrap().is_dir() {
            continue;
        }
        if process
            .file_name()
            .to_str()
            .map_or(true, |n| n.contains(|c: char| !c.is_numeric()))
        {
            continue;
        }
        let mut stat = String::new();
        File::open(process.path().join("stat"))
            .unwrap()
            .read_to_string(&mut stat)
            .unwrap();
        if stat.split(' ').nth(1) == Some("(ffxiv_dx11.exe)") {
            return Some(process.path());
        }
    }
    None
}

fn parse_hex_sockaddr(addr: &str) -> Ipv4SocketAddr {
    let (ip, port) = addr.rsplit_once(':').unwrap();
    // let ip = u32::from_be(u32::from_str_radix(ip, 16).unwrap());
    let ip = u32::from_str_radix(ip, 16).unwrap();
    let port = u16::from_str_radix(port, 16).unwrap();
    Ipv4SocketAddr { ip, port }
}

fn get_ffxiv_sockets() -> Vec<SocketAddrPair> {
    let ffxiv = find_ffxiv_proc_path().unwrap();
    let ffxiv_socket_fds = ffxiv
        .join("fd")
        .read_dir()
        .unwrap()
        .map(|fd| std::fs::metadata(fd.unwrap().path()).unwrap())
        .filter(|fd| fd.file_type().is_socket())
        .map(|fd| fd.st_ino())
        .map(|ino| ino.to_string())
        .collect::<HashSet<_>>();
    let mut tcp_conns = String::new();
    File::open(ffxiv.join("net/tcp"))
        .unwrap()
        .read_to_string(&mut tcp_conns)
        .unwrap();
    let mut tcp_conns = tcp_conns.lines();
    let headers = parse_header(tcp_conns.next().unwrap());
    tcp_conns
        .map(|conn| {
            headers
                .iter()
                .map(|header| {
                    (
                        header.name,
                        if let Some(end_col) = header.end_col {
                            &conn[header.start_col..end_col]
                        } else {
                            &conn[header.start_col..]
                        }
                        .trim()
                        .split(' ')
                        .next()
                        .unwrap_or_default(),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .filter(|conn| ffxiv_socket_fds.contains(conn["inode"]))
        .map(|conn| {
            let local = parse_hex_sockaddr(conn["local_address"]);
            let remote = parse_hex_sockaddr(conn["rem_address"]);
            SocketAddrPair { local, remote }
        })
        .collect::<Vec<_>>()
}

#[derive(Serialize)]
struct SocketAddrPair {
    local: Ipv4SocketAddr,
    remote: Ipv4SocketAddr,
}

#[derive(Serialize, Debug, Clone, Copy)]
struct Ipv4SocketAddr {
    ip: u32,
    port: u16,
}

struct HeaderField<'a> {
    name: &'a str,
    start_col: usize,
    end_col: Option<usize>,
}

fn parse_header(header: &str) -> Vec<HeaderField> {
    let mut fields = Vec::new();
    let mut current_field_start = None;
    let mut prev_was_separator = true;
    for (i, chr) in header.char_indices() {
        if chr != ' ' && prev_was_separator {
            if let Some(start_i) = current_field_start {
                fields.push(HeaderField {
                    name: header[start_i..i].trim(),
                    start_col: start_i,
                    end_col: Some(i),
                });
            }
            current_field_start = Some(i);
        }
        prev_was_separator = chr == ' ';
    }
    if let Some(start_i) = current_field_start {
        fields.push(HeaderField {
            name: header[start_i..].trim(),
            start_col: start_i,
            end_col: None,
        });
    }
    fields
}

#[tokio::main]
async fn main() {
    use axum::routing::get;
    let app = Router::new().route("/sockets", get(|| async { Json(get_ffxiv_sockets()) }));
    axum::Server::bind(&"127.0.0.1:9678".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
