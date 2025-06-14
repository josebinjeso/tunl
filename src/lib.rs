mod common;
mod config;
mod proxy;

use crate::config::Config;
use crate::proxy::*;

use base64::{engine::general_purpose::URL_SAFE, Engine as _};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;
use worker::*;

#[event(fetch)]
async fn main(req: Request, env: Env, _: Context) -> Result<Response> {
    // Ambil UUID dari variabel lingkungan
    let uuid = env
        .var("UUID")
        .map(|x| Uuid::parse_str(&x.to_string()).unwrap_or_default())?;
    
    // Ambil host dari URL permintaan
    let host = req.url()?.host().map(|x| x.to_string()).unwrap_or_default();
    
    // Buat objek konfigurasi
    let config = Config { uuid, host };

    // --- LOGIKA BARU UNTUK MENANGANI PERMINTAAN HTTP VS. WEBSOCKET ---
    if req.headers().get("Upgrade").as_deref() == Some("websocket") {
        // Jika permintaan mengandung header "Upgrade: websocket",
        // itu adalah permintaan handshake WebSocket.
        // Langsung panggil fungsi tunnel dengan data konfigurasi.
        tunnel(req, RouteContext::new(req, env, config)).await // Teruskan req ke tunnel
    } else {
        // Jika bukan permintaan handshake WebSocket,
        // Worker akan mengembalikan respons HTTP biasa.
        // Kita tetap menggunakan Router untuk route "/link" atau bisa juga handle GET /
        Router::with_data(config)
            .on_async("/", |_, cx| async { 
                // Mengembalikan pesan yang ramah jika diakses langsung dari browser
                Ok(Response::ok("Hello from Tunl Worker! This worker primarily handles WebSocket connections for tunneling. If you're seeing this, your client likely isn't sending a WebSocket upgrade request.")?)
            })
            .on("/link", link) // Fungsi link tetap di handle oleh Router
            .run(req, env)
            .await
    }
    // --- AKHIR LOGIKA BARU ---
}

async fn tunnel(req: Request, cx: RouteContext<Config>) -> Result<Response> {
    // Fungsi ini sekarang hanya akan dipanggil jika permintaan sudah dipastikan adalah upgrade WebSocket
    let WebSocketPair { server, client } = WebSocketPair::new()?;

    server.accept()?; // Menerima koneksi WebSocket di sisi server

    // Spawn task lokal untuk memproses stream VMess
    wasm_bindgen_futures::spawn_local(async move {
        let events = server.events().unwrap(); // Menggunakan unwrap, pertimbangkan error handling yang lebih baik
        if let Err(e) = VmessStream::new(cx.data, &server, events).process().await {
            console_log!("[tunnel]: {}", e);
        }
    });

    // Mengembalikan client WebSocket ke klien
    Response::from_websocket(client)
}

fn link(_: Request, cx: RouteContext<Config>) -> Result<Response> {
    #[derive(Serialize)]
    struct Link {
        description: String,
        link: String,
    }

    let link = {
        let host = cx.data.host.to_string();
        let uuid = cx.data.uuid.to_string();
        let config = json!({
            "ps": "tunl",
            "v": "2",
            "add": host, // Menggunakan 'host' dari Worker itu sendiri
            "port": "443", // Defaultkan ke 443 karena menggunakan Cloudflare Worker (HTTPS)
            "id": uuid,
            "aid": "0",
            "scy": "auto", // Lebih fleksibel daripada "zero"
            "net": "ws",
            "type": "none",
            "host": host, // Host SNI, sama dengan add
            "path": "/tunnel", // Tambahkan path jika Anda ingin lebih dari sekadar root
            "tls": "tls", // PENTING: Aktifkan TLS di link config
            "sni": host, // Sama dengan host
            "alpn": "h2", // Atau "http/1.1" jika lebih umum
            "fp": "chrome" // Fingerprint opsional untuk menyamarkan
        });
        format!("vmess://{}", URL_SAFE.encode(config.to_string()))
    };

    Response::from_json(&Link {
        link,
        description:
            "visit https://scanner.github1.cloud/ and replace the IP address in the configuration with a clean one".to_string()
    })
}
