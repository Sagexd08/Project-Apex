//! Minimal strict HTTP/1.1 request reader and response writer.
use crate::{api, Server};
use std::io::{Read, Write};
use std::net::TcpStream;

pub const MAX_HEADER: usize = 16 * 1024;
pub const MAX_BODY: usize = 8 * 1024 * 1024;
const CONSOLE: &str = include_str!("console.html");

pub struct Request {
    pub method: String,
    pub path: String,
    pub bearer: Option<Vec<u8>>,
    pub body: Vec<u8>,
}

pub fn respond(s: &mut TcpStream, status: u16, ctype: &str, body: &[u8]) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    // A client that hung up is not an error worth surfacing.
    let _ = s.write_all(head.as_bytes()).and_then(|_| s.write_all(body));
}

fn find_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn read_head(s: &mut TcpStream, chunk: &mut [u8]) -> Result<(Vec<u8>, usize), u16> {
    let mut buf = Vec::with_capacity(1024);
    loop {
        if let Some(i) = find_end(&buf) {
            return Ok((buf, i));
        }
        if buf.len() > MAX_HEADER {
            return Err(413);
        }
        match s.read(chunk) {
            Ok(0) | Err(_) => return Err(400),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

/// Reads head + body. `Err(status)` is the status to answer with.
pub fn read_request(s: &mut TcpStream) -> Result<Request, u16> {
    let mut chunk = [0u8; 4096];
    let (buf, end) = read_head(s, &mut chunk)?;
    let head = std::str::from_utf8(&buf[..end]).map_err(|_| 400u16)?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next().ok_or(400u16)?.split(' ');
    let (method, path) = (first.next().ok_or(400u16)?, first.next().ok_or(400u16)?);
    let (mut len, mut bearer) = (0usize, None);
    for l in lines {
        let (k, v) = l.split_once(':').ok_or(400u16)?;
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        match k.as_str() {
            "content-length" => len = v.parse().map_err(|_| 400u16)?,
            "transfer-encoding" => return Err(400),
            "authorization" => {
                bearer = v
                    .strip_prefix("Bearer ")
                    .map(|t| t.trim().as_bytes().to_vec())
            }
            _ => {}
        }
    }
    if len > MAX_BODY {
        return Err(413);
    }
    let mut body = buf[end + 4..].to_vec();
    if body.len() > len {
        return Err(400);
    }
    while body.len() < len {
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(400),
            Ok(n) => body.extend_from_slice(&chunk[..n.min(len - body.len())]),
        }
    }
    Ok(Request {
        method: method.to_string(),
        path: path.to_string(),
        bearer,
        body,
    })
}

pub fn handle(server: &Server, mut s: TcpStream) {
    let req = match read_request(&mut s) {
        Ok(r) => r,
        Err(code) => {
            return respond(
                &mut s,
                code,
                "application/json",
                br#"{"error":"BAD_REQUEST"}"#,
            )
        }
    };
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => respond(&mut s, 200, "text/html; charset=utf-8", CONSOLE.as_bytes()),
        ("GET", "/healthz") => respond(&mut s, 200, "text/plain", b"ok"),
        _ => {
            let (code, body) = api::dispatch(server, &req);
            respond(
                &mut s,
                code,
                "application/json",
                body.to_string().as_bytes(),
            );
        }
    }
}
