//! 保留旧版开发 HTTP 通道；原生主程序不启动此服务。
use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
};
use tracing::error;

/// Starts the local HTTP + IPC bridge server serving `src_ui_dir` and `POST /api/ipc`.
pub fn serve_desktop_ui(
    host: Arc<DesktopHost>,
    listener: TcpListener,
    src_ui_dir: PathBuf,
) -> std::io::Result<()> {
    for stream_res in listener.incoming() {
        match stream_res {
            Ok(stream) => {
                let host_ref = Arc::clone(&host);
                let ui_dir = src_ui_dir.clone();
                std::thread::spawn(move || {
                    if let Err(err) = handle_http_connection(stream, &host_ref, &ui_dir) {
                        error!(error = %err, "HTTP/IPC connection error");
                    }
                });
            }
            Err(err) => {
                error!(error = %err, "TCP accept error");
            }
        }
    }
    Ok(())
}

fn handle_http_connection(
    mut stream: TcpStream,
    host: &DesktopHost,
    ui_dir: &Path,
) -> std::io::Result<()> {
    let mut buf = [0u8; 65536];
    let n = stream.read(&mut buf)?;
    if n == 0 {
        return Ok(());
    }

    let raw_req = String::from_utf8_lossy(&buf[..n]);
    let header_end = raw_req.find("\r\n\r\n").unwrap_or(raw_req.len());
    let header_part = &raw_req[..header_end];
    let mut lines = header_part.lines();
    let first_line = lines.next().unwrap_or("");
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let raw_path = parts.next().unwrap_or("/");
    let path = raw_path.split('?').next().unwrap_or("/");

    let mut content_length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse::<usize>().unwrap_or(0);
            }
        }
    }

    let body_start = if header_end + 4 <= n {
        header_end + 4
    } else {
        n
    };
    let mut body_bytes = buf[body_start..n].to_vec();
    while body_bytes.len() < content_length {
        let mut more = vec![0u8; content_length - body_bytes.len()];
        let read_more = stream.read(&mut more)?;
        if read_more == 0 {
            break;
        }
        body_bytes.extend_from_slice(&more[..read_more]);
    }

    if method == "OPTIONS" {
        return write_http_response(&mut stream, "200 OK", "text/plain", b"");
    }

    if method == "POST" && path == "/api/ipc" {
        let parsed: Result<IpcRequest, _> = serde_json::from_slice(&body_bytes);
        let resp = match parsed {
            Ok(req) => host.invoke_ipc(&req.cmd, req.args),
            Err(err) => IpcResponse::err(format!("Invalid IPC JSON request: {}", err)),
        };
        let json_bytes = serde_json::to_vec(&resp).unwrap_or_else(|_| b"{}".to_vec());
        return write_http_response(
            &mut stream,
            "200 OK",
            "application/json; charset=utf-8",
            &json_bytes,
        );
    }

    // Serve static files from `src-ui/`
    let rel_file = match path {
        "/" | "/index.html" => "index.html",
        "/styles.css" => "styles.css",
        "/app.js" => "app.js",
        _ => "index.html",
    };

    let file_path = ui_dir.join(rel_file);
    let content_type = match rel_file {
        "styles.css" => "text/css; charset=utf-8",
        "app.js" => "application/javascript; charset=utf-8",
        _ => "text/html; charset=utf-8",
    };

    match std::fs::read(&file_path) {
        Ok(bytes) => write_http_response(&mut stream, "200 OK", content_type, &bytes),
        Err(_) => write_http_response(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"404 Not Found",
        ),
    }
}

fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Content-Type\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status,
        content_type,
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}
