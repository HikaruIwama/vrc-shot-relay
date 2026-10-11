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
    let worker = thread::spawn(move || engine::run(folder, directory, url, commands_rx, ui_tx));
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
fn newly_created_photo_reaches_webhook_and_leaves_queue() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (request_tx, request_rx) = mpsc::channel();
    let server = thread::spawn(move || {
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

#[test]
fn rate_limited_photo_is_retried_and_completed() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (request_tx, request_rx) = mpsc::channel();
    let (respond_tx, respond_rx) = mpsc::channel();
    let server = thread::spawn(move || {
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
