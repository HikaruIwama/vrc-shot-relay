use image::{DynamicImage, ImageBuffer, Rgba};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use vrc_shot_relay::engine::{self, Command, UiEvent};

fn read_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let request_line = line.clone();
    let mut length = 0usize;
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    (request_line, body)
}

fn accept_verification(listener: &TcpListener) {
    let (mut stream, _) = listener.accept().unwrap();
    let (request, body) = read_request(&mut stream);
    assert_eq!(request, "GET /api/webhooks/test HTTP/1.1\r\n");
    assert!(body.is_empty());
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
        .unwrap();
}

fn test_directory(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("vrc-shot-relay-{name}-{stamp}"));
    fs::create_dir_all(root.join("photos")).unwrap();
    root
}

fn start_engine(
    root: &Path,
    port: u16,
) -> (
    Sender<Command>,
    Receiver<UiEvent>,
    JoinHandle<Result<(), String>>,
) {
    let (commands_tx, commands_rx) = mpsc::channel();
    let (ui_tx, ui_rx) = mpsc::channel();
    let url = reqwest::Url::parse(&format!("http://127.0.0.1:{port}/api/webhooks/test")).unwrap();
    let folder = root.join("photos");
    let directory = root.to_owned();
    let engine = engine::prepare(folder, directory, url).unwrap();
    let worker = thread::spawn(move || engine::run(engine, commands_rx, ui_tx));
    assert!(matches!(
        ui_rx.recv_timeout(Duration::from_secs(5)),
        Ok(UiEvent::Status(_))
    ));
    (commands_tx, ui_rx, worker)
}

fn create_photo(root: &Path) -> PathBuf {
    let path = root.join("photos/VRChat_2026-10-11_12-34-56.123_64x64.png");
    DynamicImage::ImageRgba8(ImageBuffer::from_pixel(64, 64, Rgba([1, 2, 3, 255])))
        .save(&path)
        .unwrap();
    path
}

fn pending_photos(root: &Path) -> Vec<PathBuf> {
    let text = fs::read_to_string(root.join("state.json")).unwrap();
    let state: serde_json::Value = serde_json::from_str(&text).unwrap();
    state["pending"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| PathBuf::from(value.as_str().unwrap()))
        .collect()
}

#[test]
fn rejected_webhook_prevents_startup_without_posting() {
    for status in [401, 403, 404, 429, 500] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            write!(
                stream,
                "HTTP/1.1 {status} Rejected\r\nContent-Length: 10\r\n\r\ntest-token"
            )
            .unwrap();
            request
        });
        let root = test_directory("rejected-webhook");
        let url = reqwest::Url::parse(&format!(
            "http://127.0.0.1:{port}/api/webhooks/123/test-token"
        ))
        .unwrap();

        let result = engine::prepare(root.join("photos"), root.clone(), url);

        assert!(
            matches!(result, Err(error) if error == format!("Could not verify Discord webhook: HTTP {status}. Check webhook_url in config.toml, then restart."))
        );
        let (request, body) = server.join().unwrap();
        assert_eq!(request, "GET /api/webhooks/123/test-token HTTP/1.1\r\n");
        assert!(body.is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn unreachable_webhook_prevents_startup_without_exposing_url() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let root = test_directory("unreachable-webhook");
    let url = reqwest::Url::parse(&format!(
        "http://127.0.0.1:{port}/api/webhooks/123/test-token"
    ))
    .unwrap();

    let result = engine::prepare(root.join("photos"), root.clone(), url);

    assert!(
        matches!(result, Err(error) if error == "Could not verify Discord webhook: network request failed. Check your connection, then restart.")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unresponsive_webhook_prevents_startup_with_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (finish_tx, finish_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        finish_rx.recv_timeout(Duration::from_secs(20)).unwrap();
        request
    });
    let root = test_directory("unresponsive-webhook");
    let url = reqwest::Url::parse(&format!(
        "http://127.0.0.1:{port}/api/webhooks/123/test-token"
    ))
    .unwrap();

    let result = engine::prepare(root.join("photos"), root.clone(), url);

    finish_tx.send(()).unwrap();
    let (request, body) = server.join().unwrap();
    assert_eq!(request, "GET /api/webhooks/123/test-token HTTP/1.1\r\n");
    assert!(body.is_empty());
    assert!(
        matches!(result, Err(error) if error == "Could not verify Discord webhook: request timed out. Check your connection, then restart.")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn webhook_redirect_is_rejected_without_following_it() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let destination = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let redirect_port = destination.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        write!(stream, "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{redirect_port}/test-token\r\nContent-Length: 0\r\n\r\n").unwrap();
    });
    let root = test_directory("redirect-webhook");
    let url = reqwest::Url::parse(&format!(
        "http://127.0.0.1:{port}/api/webhooks/123/test-token"
    ))
    .unwrap();

    let result = engine::prepare(root.join("photos"), root.clone(), url);

    server.join().unwrap();
    assert!(
        matches!(result, Err(error) if error == "Could not verify Discord webhook: HTTP 302. Check webhook_url in config.toml, then restart.")
    );
    destination.set_nonblocking(true).unwrap();
    assert_eq!(
        destination.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_queue_is_reported_before_watching_photos() {
    let root = test_directory("invalid-queue");
    fs::write(root.join("state.json"), "invalid JSON").unwrap();
    let url = reqwest::Url::parse("http://127.0.0.1:1/api/webhooks/test").unwrap();

    let result = engine::prepare(root.join("photos"), root.clone(), url);

    assert!(result.is_err_and(|error| error.starts_with("Invalid state file:")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_folder_is_reported_before_watching_photos() {
    let root = test_directory("missing-folder");
    let folder = root.join("missing");
    let url = reqwest::Url::parse("http://127.0.0.1:1/api/webhooks/test").unwrap();

    let result = engine::prepare(folder.clone(), root.clone(), url);

    assert!(result.is_err_and(|error| error.starts_with("Could not scan photos:")));
    assert!(!folder.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn multilayer_photo_reaches_webhook_in_one_post_and_leaves_queue() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (request_tx, request_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        accept_verification(&listener);
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
        request_tx.send(request).unwrap();
    });

    let root = test_directory("flow");
    let (commands_tx, ui_rx, worker) = start_engine(&root, port);

    create_photo(&root);
    for suffix in ["UI", "CustomLayer", "Environment"] {
        let original = root.join("photos/VRChat_2026-10-11_12-34-56.123_64x64.png");
        fs::copy(
            &original,
            original.with_file_name(format!("VRChat_2026-10-11_12-34-56.123_64x64_{suffix}.png")),
        )
        .unwrap();
    }
    let (request_line, body) = request_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    let status = ui_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    commands_tx.send(Command::Stop).unwrap();
    worker.join().unwrap().unwrap();
    server.join().unwrap();
    let pending = pending_photos(&root);

    assert!(request_line.contains("wait=true"));
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("files[0]"));
    assert!(body.contains("files[3]"));
    assert!(body.contains("allowed_mentions"));
    assert!(!body.contains("\"embeds\""));
    assert!(matches!(status, UiEvent::Status(_)));
    assert!(pending.is_empty());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rate_limited_photo_is_retried_and_completed() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (request_tx, request_rx) = mpsc::channel();
    let (respond_tx, respond_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        accept_verification(&listener);
        let (mut first, _) = listener.accept().unwrap();
        let first_request = read_request(&mut first);
        let first_response_at = Instant::now();
        first
            .write_all(
                b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0.5\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        request_tx.send((first_request, first_response_at)).unwrap();
        let (mut second, _) = listener.accept().unwrap();
        let second_request = read_request(&mut second);
        request_tx.send((second_request, Instant::now())).unwrap();
        respond_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        second
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
    });

    let root = test_directory("retry");
    let (commands_tx, ui_rx, worker) = start_engine(&root, port);

    let photo_path = create_photo(&root);
    let (first_request, first_response_at) =
        request_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    let error = ui_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (second_request, second_request_at) =
        request_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let pending_before_success = pending_photos(&root);

    assert!(matches!(error, UiEvent::Error(_)));
    assert!(second_request_at.duration_since(first_response_at) >= Duration::from_millis(450));
    assert!(String::from_utf8_lossy(&first_request.1).contains("files[0]"));
    assert!(String::from_utf8_lossy(&second_request.1).contains("files[0]"));
    assert_eq!(first_request.0, second_request.0);
    assert_eq!(pending_before_success, vec![photo_path]);

    respond_tx.send(()).unwrap();
    let status = ui_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    commands_tx.send(Command::Stop).unwrap();
    worker.join().unwrap().unwrap();
    server.join().unwrap();
    let pending_after_success = pending_photos(&root);

    assert!(matches!(status, UiEvent::Status(_)));
    assert!(pending_after_success.is_empty());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn newly_created_photo_reaches_webhook_and_leaves_queue() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (request_tx, request_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        accept_verification(&listener);
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
        request_tx.send(request).unwrap();
    });

    let root = test_directory("flow");
    let (commands_tx, ui_rx, worker) = start_engine(&root, port);

    create_photo(&root);
    let (request_line, body) = request_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    let status = ui_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    commands_tx.send(Command::Stop).unwrap();
    worker.join().unwrap().unwrap();
    server.join().unwrap();
    let pending = pending_photos(&root);

    assert!(request_line.contains("wait=true"));
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("files[0]"));
    assert!(body.contains("allowed_mentions"));
    assert!(!body.contains("\"embeds\""));
    assert!(matches!(status, UiEvent::Status(_)));
    assert!(pending.is_empty());

    fs::remove_dir_all(root).unwrap();
}
