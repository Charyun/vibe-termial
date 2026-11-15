use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use vibe_terminal::web::Server;

fn fetch(address: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut text = String::new();
    stream.read_to_string(&mut text).unwrap();
    text
}

#[test]
fn served_page_has_session_picker_and_mobile_layout() {
    let server = Server::start(
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        Arc::new(|_| Ok(())),
    )
    .unwrap();
    let html = fetch(server.address, "/");
    for marker in [
        "id=\"picker\"",
        "id=\"picker-list\"",
        "id=\"session-view\"",
        "id=\"back-to-picker\"",
        "id=\"picker-logout\"",
    ] {
        assert!(html.contains(marker), "HTML 缺少 {marker}");
    }
    let js = fetch(server.address, "/app.js");
    for marker in [
        "renderPicker",
        "showPicker",
        "showSession",
        "mobileLayout",
        "function fit",
    ] {
        assert!(js.contains(marker), "JS 缺少 {marker}");
    }
    let css = fetch(server.address, "/style.css");
    for marker in [
        ".picker-session",
        "#back-to-picker",
        "#terminal{width:auto}",
    ] {
        assert!(css.contains(marker), "CSS 缺少 {marker}");
    }
    println!(
        "served page ok: html {} bytes, js {} bytes, css {} bytes",
        html.len(),
        js.len(),
        css.len()
    );
}
