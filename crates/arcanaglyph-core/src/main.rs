// crates/arcanaglyph-core/src/main.rs

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures_util::{SinkExt, StreamExt};
use log::info;
// ИСПРАВЛЕНИЕ: Добавляем стандартные модули для работы с ФС и путями
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::broadcast;
use tungstenite::Message;
use vosk::{Model, Recognizer};

fn record_and_transcribe_with_stop(
    stop_rx: std_mpsc::Receiver<()>,
    recognizer_arc: Arc<Mutex<Recognizer>>,
) -> String {
    info!("Начинаю запись...");
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .expect("Нет доступного устройства ввода");
    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(48000),
        buffer_size: cpal::BufferSize::Default,
    };
    let recognizer_clone = Arc::clone(&recognizer_arc);
    let stream = device
        .build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                let mut rec = recognizer_clone.lock().unwrap();
                rec.accept_waveform(data).unwrap();
            },
            |err| eprintln!("Ошибка в аудиопотоке: {}", err),
            None,
        )
        .expect("Не удалось создать аудиопоток");
    stream.play().expect("Не удалось запустить аудиопоток");
    info!("Идет запись... (нажмите хоткей для останова или ждите таймаут)");

    let _ = stop_rx.recv();
    stream.pause().expect("Не удалось остановить аудиопоток");
    info!("Запись завершена. Начинаю транскрибацию...");

    let mut recognizer_guard = recognizer_arc.lock().unwrap();
    let final_result_json = recognizer_guard.final_result().single().unwrap();
    info!("Финальный результат: {}", final_result_json.text);
    let result_text = final_result_json.text.to_string();
    recognizer_guard.reset();
    result_text
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    addr: SocketAddr,
    _recognizer: Arc<Mutex<Recognizer>>,
    result_tx: Arc<broadcast::Sender<String>>,
) {
    info!("GUI подключился: {}", addr);
    let ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .expect("Ошибка при рукопожатии websocket");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let mut result_rx = result_tx.subscribe();

    loop {
        tokio::select! {
            result = result_rx.recv() => {
                match result {
                    Ok(msg) => {
                        let msg_text = Message::Text(msg.into());
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
    env_logger::init();
    info!("Инициализация...");

    // --- ИСПРАВЛЕНИЕ: Создание абсолютного, канонического пути к модели ---
    let mut model_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    model_path.push("../../models/vosk-model-ru-0.42");

    // Превращаем относительный путь в абсолютный.
    let canonical_model_path = fs::canonicalize(&model_path)
        .expect("Не удалось найти директорию с моделью по вычисленному пути");

    info!("Загрузка модели из: {:?}", canonical_model_path);
    let model =
        Model::new(canonical_model_path.to_str().unwrap()).expect("Не удалось создать модель");
    info!("Модель успешно загружена.");
    let recognizer = Arc::new(Mutex::new(
        Recognizer::new(&model, 48000.0).expect("Не удалось создать распознаватель"),
    ));

    let is_busy = Arc::new(tokio::sync::Mutex::new(false));
    let current_stop_tx = Arc::new(tokio::sync::Mutex::new(
        Option::<std_mpsc::Sender<()>>::None,
    ));
    let (result_bcast_tx, _) = broadcast::channel::<String>(32);
    let result_tx = Arc::new(result_bcast_tx);

    let tcp_listener = TcpListener::bind("127.0.0.1:9001").await.unwrap();
    info!("Сервер сокетов запущен...");
    info!("Слушаю триггеры на UDP порту 9002.");

    let is_busy_udp = Arc::clone(&is_busy);
    let current_stop_tx_udp = Arc::clone(&current_stop_tx);
    let result_tx_udp = Arc::clone(&result_tx);
    let recognizer_udp = Arc::clone(&recognizer);
    tokio::spawn(async move {
        let udp_socket = UdpSocket::bind("127.0.0.1:9002")
            .await
            .expect("Failed to bind UDP");
        let mut buf = [0u8; 1024];

        loop {
            if let Ok((n, _)) = udp_socket.recv_from(&mut buf).await {
                let trigger_str = String::from_utf8_lossy(&buf[0..n]);
                if !trigger_str.contains("trigger") {
                    continue;
                }

                let mut busy_guard = is_busy_udp.lock().await;

                if *busy_guard {
                    let mut stop_tx_guard = current_stop_tx_udp.lock().await;
                    if let Some(tx) = stop_tx_guard.take() {
                        info!("Получен триггер для остановки записи.");
                        let _ = tx.send(());
                    } else {
                        info!("Игнорирую триггер, идет обработка...");
                    }
                } else {
                    info!("Получен триггер для начала записи.");
                    *busy_guard = true;

                    let (local_stop_tx, local_stop_rx) = std_mpsc::channel();
                    *current_stop_tx_udp.lock().await = Some(local_stop_tx);

                    let _ = result_tx_udp.send(
                        serde_json::json!({
                            "type": "status",
                            "status": "recording_started"
                        })
                        .to_string(),
                    );

                    let stop_tx_for_timer = Arc::clone(&current_stop_tx_udp);
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(20)).await;
                        if let Some(tx) = stop_tx_for_timer.lock().await.take() {
                            info!("Запись останавливается по таймеру (20с).");
                            let _ = tx.send(());
                        }
                    });

                    let result_tx_clone = Arc::clone(&result_tx_udp);
                    let recognizer_clone = Arc::clone(&recognizer_udp);
                    let stop_tx_for_recorder = Arc::clone(&current_stop_tx_udp);
                    let is_busy_clone = Arc::clone(&is_busy_udp);
                    tokio::spawn(async move {
                        let text_future = tokio::task::spawn_blocking(move || {
                            record_and_transcribe_with_stop(local_stop_rx, recognizer_clone)
                        });

                        match text_future.await {
                            Ok(text_result) => {
                                let msg = serde_json::json!({
                                    "type": "transcription_result",
                                    "text": text_result
                                })
                                .to_string();
                                let _ = result_tx_clone.send(msg);
                            }
                            Err(e) => {
                                eprintln!("Задача записи завершилась с ошибкой: {:?}", e);
                            }
                        }

                        info!("Обработка завершена. Система готова к новой записи.");
                        let _ = result_tx_clone.send(
                            serde_json::json!({
                                "type": "status",
                                "status": "finished_processing"
                            })
                            .to_string(),
                        );

                        *is_busy_clone.lock().await = false;
                        *stop_tx_for_recorder.lock().await = None;
                    });
                }
            }
        }
    });

    while let Ok((stream, addr)) = tcp_listener.accept().await {
        tokio::spawn(handle_connection(
            stream,
            addr,
            Arc::clone(&recognizer),
            Arc::clone(&result_tx),
        ));
    }
}
