// crates/arcanaglyph-core/src/main.rs
// Legacy standalone-сервер (для отладки без Tauri)

use arcanaglyph_core::{ArcanaEngine, CoreConfig, EngineEvent};
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::net::{TcpListener, UdpSocket};
use tracing::info;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use tungstenite::Message;

async fn handle_connection(
    stream: tokio::net::TcpStream,
    addr: SocketAddr,
    mut event_rx: broadcast::Receiver<EngineEvent>,
) {
    info!("GUI подключился: {}", addr);

    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::error!("Ошибка при рукопожатии websocket: {}", e);
            return;
        }
    };

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    loop {
        tokio::select! {
            result = event_rx.recv() => {
                match result {
                    Ok(event) => {
                        let msg_json = match &event {
                            EngineEvent::RecordingStarted => {
                                serde_json::json!({"type": "status", "status": "recording_started"})
                            }
                            EngineEvent::TranscriptionResult(text) => {
                                serde_json::json!({"type": "transcription_result", "text": text})
                            }
                            EngineEvent::RecordingPaused => {
                                serde_json::json!({"type": "status", "status": "recording_paused"})
                            }
                            EngineEvent::RecordingResumed => {
                                serde_json::json!({"type": "status", "status": "recording_resumed"})
                            }
                            EngineEvent::FinishedProcessing => {
                                serde_json::json!({"type": "status", "status": "finished_processing"})
                            }
                            EngineEvent::RequestFocus => {
                                continue;
                            }
                            EngineEvent::Error(msg) => {
                                serde_json::json!({"type": "error", "message": msg})
                            }
                        };
                        let msg_text = Message::Text(msg_json.to_string().into());
                        if ws_sender.send(msg_text).await.is_err() {
                            info!("Не удалось отправить сообщение, клиент {} отключился.", addr);
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("Канал результатов закрыт.");
                        break;
                    }
                }
            }
            Some(_) = ws_receiver.next() => {
                info!("Клиент {} отключился.", addr);
                break;
            }
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    info!("Инициализация...");

    let config = CoreConfig::default();
    // Legacy-режим: окно всегда "скрыто", текст вставляется через enigo
    let window_visible = Arc::new(AtomicBool::new(false));
    let engine = Arc::new(ArcanaEngine::new(config, window_visible).expect("Не удалось инициализировать движок"));

    let tcp_listener = TcpListener::bind("127.0.0.1:9001")
        .await
        .expect("Не удалось привязать TCP :9001");
    info!("Сервер сокетов запущен...");
    info!("Слушаю триггеры на UDP порту 9002.");

    // UDP-триггер
    let engine_udp = Arc::clone(&engine);
    tokio::spawn(async move {
        let udp_socket = UdpSocket::bind("127.0.0.1:9002")
            .await
            .expect("Не удалось привязать UDP :9002");
        let mut buf = [0u8; 1024];

        loop {
            if let Ok((n, _)) = udp_socket.recv_from(&mut buf).await {
                let trigger_str = String::from_utf8_lossy(&buf[0..n]);
                if trigger_str.contains("trigger") {
                    engine_udp.trigger();
                }
            }
        }
    });

    // WebSocket-сервер
    while let Ok((stream, addr)) = tcp_listener.accept().await {
        let event_rx = engine.subscribe();
        tokio::spawn(handle_connection(stream, addr, event_rx));
    }
}
