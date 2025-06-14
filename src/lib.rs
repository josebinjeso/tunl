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
    let uuid = env
        .var("UUID")
        .map(|x| Uuid::parse_str(&x.to_string()).unwrap_or_default())?;
    
    let host = req.url()?.host().map(|x| x.to_string()).unwrap_or_default();
    
    let config = Config { uuid, host };

    // Router akan mengarahkan semua permintaan, termasuk yang ke '/'
    // Logika pengecekan WebSocket akan dipindahkan ke dalam fungsi `tunnel`
    Router::with_data(config)
        .on_async("/", tunnel) // `tunnel` akan menangani permintaan ke root ("/")
        .on("/link", link)    // `link` akan menangani permintaan ke "/link"
        .run(req, env)
        .await
}

// Fungsi tunnel sekarang akan menerima permintaan dan memeriksa apakah itu WebSocket
async fn tunnel(req: Request, cx: RouteContext<Config>) -> Result<Response> {
    // Memperbaiki error `as_deref` dengan menggunakan `?`
    // Jika header "Upgrade" ada dan nilainya "websocket", ini adalah handshake WebSocket.
    if req.headers().get("Upgrade")?.as_deref() == Some("websocket") {
        // Ini adalah permintaan upgrade WebSocket yang valid
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
    } else { // Ini adalah bagian `else` dari `if` di atas
        // Ini adalah permintaan HTTP biasa (misalnya, dari browser atau klien non-WebSocket)
        // Kembalikan respons HTTP yang ramah atau informatif
        Ok(Response::ok(
            "Hello from Tunl Worker! This endpoint primarily handles WebSocket connections for tunneling. \
             If you're seeing this, your client likely isn't sending a WebSocket upgrade request. \
             Try accessing /link for VMess configuration."
        )?)
        // Atau, jika Anda ingin lebih ketat:
        // Ok(Response::error("Bad Request: Expected WebSocket upgrade header", 400)?)
    }
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
            "path": "/", // Biarkan path kosong atau "/" jika Worker mendengarkan di root
            "tls": "tls", // PENTING: Aktifkan TLS di link config
            "sni": host, // Sama dengan host
            "alpn": ["http/1.1", "h2"], // Bisa lebih dari satu, klien akan memilih
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
