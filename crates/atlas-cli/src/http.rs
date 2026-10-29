use crate::{api, Server};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub const MAX_HEADER: usize = 16 * 1024;
pub const MAX_BODY: usize = 8 * 1024 * 1024;
pub const REQUEST_DEADLINE: Duration = Duration::from_secs(15);
const READ_SLICE: Duration = Duration::from_secs(5);
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
        408 => "Request Timeout",
        413 => "Payload Too Large",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes()).and_then(|_| s.write_all(body));
}

fn find_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn arm(s: &TcpStream, started: Instant) -> Result<(), u16> {
    let left = REQUEST_DEADLINE
        .checked_sub(started.elapsed())
        .filter(|d| !d.is_zero())
        .ok_or(408u16)?;
    s.set_read_timeout(Some(left.min(READ_SLICE))).ok();
    Ok(())
}

fn read_head(
    s: &mut TcpStream,
    chunk: &mut [u8],
    started: Instant,
) -> Result<(Vec<u8>, usize), u16> {
    let mut buf = Vec::with_capacity(1024);
    loop {
        if let Some(i) = find_end(&buf) {
            return Ok((buf, i));
        }
        if buf.len() > MAX_HEADER {
            return Err(413);
        }
        arm(s, started)?;
        match s.read(chunk) {
            Ok(0) => return Err(400),
            Err(e) if is_timeout(&e) => return Err(408),
            Err(_) => return Err(400),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

pub fn read_request(s: &mut TcpStream) -> Result<Request, u16> {
    let started = Instant::now();
    let mut chunk = [0u8; 4096];
    let (buf, end) = read_head(s, &mut chunk, started)?;
    let head = std::str::from_utf8(&buf[..end]).map_err(|_| 400u16)?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next().ok_or(400u16)?.split(' ');
    let (method, path) = (first.next().ok_or(400u16)?, first.next().ok_or(400u16)?);
    let (mut len, mut bearer, mut saw_len) = (0usize, None, false);
    for l in lines {
        let (k, v) = l.split_once(':').ok_or(400u16)?;
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        match k.as_str() {
            "content-length" => {
                if saw_len {
                    return Err(400);
                }
                saw_len = true;
                len = v.parse().map_err(|_| 400u16)?
            }
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
        arm(s, started)?;
        match s.read(&mut chunk) {
            Ok(0) => return Err(400),
            Err(e) if is_timeout(&e) => return Err(408),
            Err(_) => return Err(400),
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
