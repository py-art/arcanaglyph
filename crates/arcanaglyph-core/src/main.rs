// crates/arcanaglyph-core/src/main.rs

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures_util::{SinkExt, StreamExt};
use std::env;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, UdpSocket}; // <-- Используем асинхронные версии
use tokio::sync::mpsc; // <-- Используем асинхронный канал
use tungstenite::Message;
use vosk::{Model, Recognizer}; // <-- Нужны для работы с потоком веб-сокета

// Функция record_and_transcribe остается почти без изменений
fn record_and_transcribe(recognizer_arc: Arc<Mutex<Recognizer>>) -> String {
    // ... (весь код функции остается прежним)
    println!("Начинаю запись...");
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
        .unwrap();
    stream.play().unwrap();
    println!("Идет запись... (5 секунд)");
    std::thread::sleep(std::time::Duration::from_secs(5));
    stream.pause().unwrap();
    println!("Запись завершена.");
    let mut recognizer_guard = recognizer_arc.lock().unwrap();
    let final_result_json = recognizer_guard.final_result().single().unwrap();
    println!("Финальный результат: {}", final_result_json.text);
    let result_text = final_result_json.text.to_string();
    recognizer_guard.reset();
    result_text
}

// Новая асинхронная функция для обработки одного клиента
async fn handle_connection(
    stream: tokio::net::TcpStream,
    addr: SocketAddr,
    recognizer: Arc<Mutex<Recognizer>>,
    trigger_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<()>>>,
) {
    println!("GUI подключился: {}", addr);
    let ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .expect("Ошибка при рукопожатии websocket");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    let mut trigger_rx = trigger_rx.lock().await;

    loop {
        tokio::select! {
            // Ждем сообщения от UDP триггера
            Some(_) = trigger_rx.recv() => {
                println!("Триггер получен!");

                // Выполняем блокирующую операцию в отдельном потоке, чтобы не блокировать tokio
                let rec_clone = Arc::clone(&recognizer);
                let text = tokio::task::spawn_blocking(move || {
                    record_and_transcribe(rec_clone)
                }).await.unwrap();

                let msg = serde_json::json!({
                    "type": "transcription_result",
                    "text": text
                }).to_string();

                if ws_sender.send(Message::Text(msg.into())).await.is_err() {
                    println!("Не удалось отправить сообщение, клиент {} отключился.", addr);
                    break;
                }
                 println!("Результат отправлен в GUI. Снова слушаю...");
            }

            // Ждем сообщения от клиента (например, о закрытии)
            Some(_) = ws_receiver.next() => {
                println!("Клиент {} отключился.", addr);
                break;
            }
        }
    }
}

// Главная функция теперь асинхронная
#[tokio::main]
async fn main() {
    // --- Инициализация ---
    println!("Инициализация...");
    let mut model_path = env::current_dir().expect("Не удалось получить текущую директорию");
    model_path.push("models/vosk-model-ru-0.42");
    println!("Загрузка модели из: {:?}", model_path);
    let model = Model::new(model_path.to_str().unwrap()).expect("Не удалось создать модель");
    println!("Модель успешно загружена.");
    let recognizer = Arc::new(Mutex::new(
        Recognizer::new(&model, 48000.0).expect("Не удалось создать распознаватель"),
    ));

    // --- Запуск слушателей ---
    let tcp_listener = TcpListener::bind("127.0.0.1:9001").await.unwrap();
    println!("Сервер сокетов запущен...");

    let udp_socket = UdpSocket::bind("127.0.0.1:9002").await.unwrap();
    println!("Слушаю триггеры на UDP порту 9002.");

    // Создаем канал, чтобы передавать сигналы от UDP в обработчик клиента
    let (trigger_tx, trigger_rx) = mpsc::channel(1);
    let trigger_rx = Arc::new(tokio::sync::Mutex::new(trigger_rx));

    // Асинхронный поток для UDP
    tokio::spawn(async move {
        let mut buf = [0; 10];
        loop {
            if udp_socket.recv_from(&mut buf).await.is_ok() {
                // Просто отправляем пустой сигнал в канал
                let _ = trigger_tx.send(()).await;
            }
        }
    });

    // --- Главный цикл сервера ---
    while let Ok((stream, addr)) = tcp_listener.accept().await {
        tokio::spawn(handle_connection(
            stream,
            addr,
            Arc::clone(&recognizer),
            Arc::clone(&trigger_rx),
        ));
    }
}
