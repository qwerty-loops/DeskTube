//! LAN phone remote: a tiny HTTP server that serves a self-contained remote
//! page and forwards commands into the app as `remote-command` events.
//!
//! Security: bound to the LAN but every request requires a per-launch token
//! (`?t=`), so only a device that scanned the QR can control playback.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

#[derive(Clone, Default, serde::Serialize)]
pub struct RemoteSnapshot {
    pub status: String,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub title: String,
    pub artist: String,
    pub thumbnail: String,
}

static SNAPSHOT: Mutex<Option<RemoteSnapshot>> = Mutex::new(None);
static TOKEN: Mutex<String> = Mutex::new(String::new());
static PORT: AtomicU64 = AtomicU64::new(0);

/// Minimal token generator: address + time mixed, good enough for pairing.
fn new_token() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// LAN IP without sending traffic: a UDP connect picks the outbound interface.
fn lan_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip().to_string())
}

fn authorized(path: &str) -> bool {
    let token = TOKEN.lock().map(|t| t.clone()).unwrap_or_default();
    !token.is_empty() && path.contains(&format!("t={token}"))
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn handle(stream: &mut TcpStream, app: &AppHandle) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    // Consume headers; capture Content-Length for POST bodies.
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
            break;
        }
        if let Some(v) = line
            .to_ascii_lowercase()
            .strip_prefix("content-length:")
            .and_then(|v| v.trim().parse().ok())
        {
            content_length = v;
        }
    }

    let route = path.split('?').next().unwrap_or("/");
    match (method.as_str(), route) {
        ("GET", "/") => {
            if !authorized(&path) {
                respond(stream, "404 Not Found", "text/plain", b"not found");
                return;
            }
            respond(
                stream,
                "200 OK",
                "text/html; charset=utf-8",
                REMOTE_PAGE.as_bytes(),
            );
        }
        ("GET", "/state") => {
            if !authorized(&path) {
                respond(stream, "404 Not Found", "text/plain", b"not found");
                return;
            }
            let snapshot = SNAPSHOT
                .lock()
                .ok()
                .and_then(|s| s.clone())
                .unwrap_or_default();
            let body = serde_json::to_vec(&snapshot).unwrap_or_default();
            respond(stream, "200 OK", "application/json", &body);
        }
        ("POST", "/cmd") => {
            if !authorized(&path) {
                respond(stream, "404 Not Found", "text/plain", b"not found");
                return;
            }
            let mut body = vec![0_u8; content_length.min(4096)];
            let _ = reader.read_exact(&mut body);
            if let Ok(cmd) = serde_json::from_slice::<serde_json::Value>(&body) {
                let _ = app.emit("remote-command", cmd);
            }
            respond(stream, "200 OK", "application/json", b"{}");
        }
        ("OPTIONS", _) => {
            respond(stream, "204 No Content", "text/plain", b"");
        }
        _ => respond(stream, "404 Not Found", "text/plain", b"not found"),
    }
}

pub fn update_state(status: &str, position_ms: u64, duration_ms: u64) {
    if let Ok(mut snap) = SNAPSHOT.lock() {
        let snap = snap.get_or_insert_with(RemoteSnapshot::default);
        snap.status = status.to_string();
        snap.position_ms = position_ms;
        snap.duration_ms = duration_ms;
    }
}

pub fn update_track(title: &str, artist: &str, thumbnail: Option<&str>) {
    if let Ok(mut snap) = SNAPSHOT.lock() {
        let snap = snap.get_or_insert_with(RemoteSnapshot::default);
        snap.title = title.to_string();
        snap.artist = artist.to_string();
        snap.thumbnail = thumbnail.unwrap_or_default().to_string();
    }
}

/// Starts the remote server; returns the pairing URL once bound.
pub fn start(app: AppHandle) {
    *TOKEN.lock().unwrap() = new_token();
    let listener = match TcpListener::bind("0.0.0.0:0") {
        Ok(l) => l,
        Err(_) => return, // no listener → remote silently unavailable
    };
    if let Ok(addr) = listener.local_addr() {
        PORT.store(addr.port() as u64, Ordering::SeqCst);
    }
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let app = app.clone();
            thread::spawn(move || {
                let mut stream = stream;
                handle(&mut stream, &app);
            });
        }
    });
}

/// Pairing info for the frontend: URL + QR SVG markup.
pub fn pairing() -> Option<serde_json::Value> {
    let port = PORT.load(Ordering::SeqCst);
    let ip = lan_ip()?;
    if port == 0 {
        return None;
    }
    let token = TOKEN.lock().ok()?.clone();
    let url = format!("http://{ip}:{port}/?t={token}");
    let qr_svg = qrcode::QrCode::new(url.as_bytes()).ok().map(|code| {
        code.render::<qrcode::render::svg::Color>()
            .min_dimensions(180, 180)
            .dark_color(qrcode::render::svg::Color("#ffffff"))
            .light_color(qrcode::render::svg::Color("#1b1d24"))
            .build()
    });
    Some(serde_json::json!({ "url": url, "qr_svg": qr_svg }))
}

const REMOTE_PAGE: &str = r##"<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>DeskTube Remote</title>
<style>
:root{--accent:#1ed760}
*{box-sizing:border-box;margin:0}
body{background:#0c0d10;color:#fff;font-family:'Segoe UI',system-ui,sans-serif;display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:100vh;padding:24px;user-select:none}
img.art{width:min(64vw,320px);aspect-ratio:1;border-radius:14px;object-fit:cover;box-shadow:0 16px 48px #0009;background:#1b1d24}
.title{margin-top:20px;font-size:19px;font-weight:700;text-align:center;max-width:80vw;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.artist{margin-top:4px;font-size:14px;color:#9a9da8;text-align:center;max-width:80vw;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.progress{width:min(80vw,360px);margin-top:22px}
.times{display:flex;justify-content:space-between;font-size:11px;color:#9a9da8;margin-top:4px}
input[type=range]{-webkit-appearance:none;width:100%;height:5px;border-radius:3px;background:#3a3d46;outline:none}
input[type=range]::-webkit-slider-thumb{-webkit-appearance:none;width:14px;height:14px;border-radius:50%;background:#fff;cursor:pointer}
.controls{display:flex;align-items:center;gap:26px;margin-top:18px}
button{background:none;border:0;color:#fff;cursor:pointer;padding:8px}
button svg{width:30px;height:30px;fill:currentColor}
button.play{width:64px;height:64px;border-radius:50%;background:var(--accent);color:#000;display:grid;place-items:center}
button.play svg{width:30px;height:30px;fill:#000}
.volume{width:min(80vw,360px);display:flex;align-items:center;gap:10px;margin-top:20px;color:#9a9da8}
.volume svg{width:18px;height:18px;fill:currentColor;flex:none}
.status{color:#666;font-size:12px;margin-top:26px}
</style></head><body>
<img class="art" id="art" alt="">
<div class="title" id="title">Nothing playing</div>
<div class="artist" id="artist">Open DeskTube and play a song</div>
<div class="progress"><input type="range" id="seek" min="0" max="1000" value="0"><div class="times"><span id="cur">0:00</span><span id="dur">0:00</span></div></div>
<div class="controls">
<button id="prev"><svg viewBox="0 0 24 24"><path d="M6 5h3v14H6zM20 5v14L9.5 12z"/></svg></button>
<button class="play" id="play"><svg viewBox="0 0 24 24"><path d="M8 5v14l11-7z"/></svg></button>
<button id="next"><svg viewBox="0 0 24 24"><path d="M15 5h3v14h-3zM4 5v14l10.5-7z"/></svg></button>
</div>
<div class="volume"><svg viewBox="0 0 24 24"><path d="M4 10v4h4l5 4V6l-5 4zM17 9a4 4 0 0 1 0 6"/></svg><input type="range" id="vol" min="0" max="200" value="80"></div>
<div class="status" id="status">connected</div>
<script>
const t=new URLSearchParams(location.search).get('t');
const q=u=>u+(u.includes('?')?'&':'?')+'t='+t;
const $=id=>document.getElementById(id);
let state={position_ms:0,duration_ms:0},scrubbing=false;
const fmt=ms=>{const s=Math.floor(ms/1000);return Math.floor(s/60)+':'+String(s%60).padStart(2,'0')};
async function poll(){
 try{
  const r=await fetch(q('/state'),{cache:'no-store'});
  if(!r.ok)throw 0;
  state=await r.json();
  $('status').textContent='connected';
  $('title').textContent=state.title||'Nothing playing';
  $('artist').textContent=state.artist||'';
  if(state.thumbnail&&$('art').src!==state.thumbnail)$('art').src=state.thumbnail;
  $('play').innerHTML=state.status==='playing'
   ?'<svg viewBox="0 0 24 24"><path d="M7 5h4v14H7zM14 5h4v14h-4z"/></svg>'
   :'<svg viewBox="0 0 24 24"><path d="M8 5v14l11-7z"/></svg>';
  if(!scrubbing&&state.duration_ms)$('seek').value=Math.round(state.position_ms/state.duration_ms*1000);
  $('cur').textContent=fmt(state.position_ms);$('dur').textContent=fmt(state.duration_ms);
 }catch(e){$('status').textContent='disconnected'}
}
async function cmd(c){try{await fetch(q('/cmd'),{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(c)})}catch(e){}}
$('play').onclick=()=>cmd({action:'toggle'});
$('prev').onclick=()=>cmd({action:'previous'});
$('next').onclick=()=>cmd({action:'next'});
$('seek').addEventListener('pointerdown',()=>scrubbing=true);
$('seek').addEventListener('change',e=>{scrubbing=false;const ms=Math.round(e.target.value/1000*state.duration_ms);cmd({action:'seek',position_ms:ms})});
$('vol').addEventListener('input',e=>cmd({action:'volume',volume:e.target.value/100}));
setInterval(poll,1000);poll();
</script></body></html>"##;
