use std::str;
use std::error;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;
use env_logger::Builder;
use futures_util::sink::SinkExt;
use log::LevelFilter;
use notify::{raw_watcher, RawEvent, RecursiveMode, Watcher};
use structopt::StructOpt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite;

#[derive(StructOpt)]
struct Cli {
    #[structopt(parse(from_os_str))]
    file_path: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug)]
enum FileChangeOperation {
    Chmod = 1,
    CloseWrite,
    Create,
    Remove,
    Rename,
    Rescan,
    Write,
    Unknown,
}

#[derive(Clone, Debug)]
struct FileChange {
    path: String,
    operation: FileChangeOperation,
}

type Tx = mpsc::Sender<Vec<FileChange>>;

#[derive(Clone, Debug)]
struct Client {
    addr: SocketAddr,
    tx: Tx,
}

type Listeners = Arc<Mutex<Vec<Client>>>;

impl From<notify::Op> for FileChangeOperation {
    fn from(source: notify::Op) -> FileChangeOperation {
        match source {
            notify::Op::CHMOD => FileChangeOperation::Chmod,
            notify::Op::CLOSE_WRITE => FileChangeOperation::CloseWrite,
            notify::Op::CREATE => FileChangeOperation::Create,
            notify::Op::REMOVE => FileChangeOperation::Remove,
            notify::Op::RENAME => FileChangeOperation::Rename,
            notify::Op::RESCAN => FileChangeOperation::Rescan,
            notify::Op::WRITE => FileChangeOperation::Write,
            _ => FileChangeOperation::Unknown,
        }
    }
}

impl FileChange {
    fn new(op: notify::Op, path: String) -> Self {
        FileChange {
            path,
            operation: FileChangeOperation::from(op),
        }
    }
}

async fn spawn_filewatcher(
    tx: broadcast::Sender<Vec<FileChange>>,
    file_path: String,
) -> Result<(), Box<dyn error::Error>> {
    tokio::spawn(async move {
        let (watcher_tx, watcher_rx) = channel();
        let mut watcher = raw_watcher(watcher_tx).expect("Error: Initializing Raw Watcher");
        watcher
            .watch(file_path, RecursiveMode::Recursive)
            .expect("Error: Watcher");

        log::info!("File watcher started...");

        let mut changes: Vec<FileChange> = vec![];
        let mut last_change: Option<Instant> = None;

        loop {
            match watcher_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(RawEvent {
                    path: Some(path),
                    op: Ok(op),
                    cookie: _cookie,
                }) => {
                    changes.push(FileChange::new(op, path.to_str().unwrap().to_string()));
                }
                Ok(event) => log::error!("broken event: {:?}", event),
                Err(e) => match e {
                    RecvTimeoutError::Timeout => (),
                    RecvTimeoutError::Disconnected => {
                        log::error!("Notify disconnected");
                        break;
                    }
                },
            };

            if changes.len() > 0 {
                let should_send = match last_change {
                    Some(t) => Instant::now().duration_since(t) > Duration::from_secs(1),
                    None => {
                        last_change = Some(Instant::now());
                        false
                    }
                };
                if !should_send {
                    continue;
                }

                log::info!("Sending changes...");
                tx.send(changes).unwrap();
                changes = vec![];
                last_change = None;
            }
        }
    });

    Ok(())
}

async fn handle_connection(listeners: Listeners, stream: TcpStream, addr: SocketAddr) {
    log::info!("Websocket server: got a connection from: {}", addr);

    let mut buf = [0; 100];
    let _len = stream.peek(&mut buf).await.expect("peek failed");
    let bufstr = str::from_utf8(&buf).unwrap();
    
    // check `bufstr` for "livereload.js"; serve it up if we get it 
    // println!("buf: {:?}", bufstr);
    if bufstr.contains("livereload.js") {
        let bytes = include_bytes!("../static/livereload.js");
        let contents = String::from_utf8_lossy(bytes);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            contents.len(),
            contents,
        );

        println!("response: {}", response);
        
        // stream.write(response.as_bytes()).expect("Failed to write response");
        // stream.flush().unwrap();
    }

    // change this to accept HTTP requests as per the livereload protocol 
    // need to serve up the livereload.js script through the same websocket 
    // or the same tcp stream 
    let mut ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .expect("Error during handshake occurred.");

    log::info!("Websocket server: connection established with {}", addr);

    let (tx, mut rx) = mpsc::channel(10);

    listeners.lock().unwrap().push(Client { addr, tx });

    while let Some(changes) = rx.recv().await {
        for change in changes {
            ws_stream
                .send(tungstenite::Message::Text(change.path))
                .await
                .unwrap();
        }
    }

    log::info!("Websocket server: {} disconnected", addr);
}

async fn spawn_websocket(listeners: Listeners) {
    // TODO: if this panics, kill the whole server 
    let listener = TcpListener::bind("127.0.0.1:35729").await.unwrap();

    log::info!("Websocket server listening on 35729");

    while let Ok((stream, addr)) = listener.accept().await {
        tokio::spawn(handle_connection(listeners.clone(), stream, addr));
    }
}

async fn run() -> Result<(), Box<dyn error::Error>> {
    let args = Cli::from_args();

    let file_path = match args.file_path {
        Some(fp) => fp.to_str().unwrap().to_string(),
        None => ".".to_string(),
    };

    let listeners = Arc::new(Mutex::new(Vec::new()));

    // TODO: Change `channel`s to be unbounded channels 
    let (btx, mut rx): (
        broadcast::Sender<Vec<FileChange>>,
        broadcast::Receiver<Vec<FileChange>>,
    ) = broadcast::channel(10);

    // spawns a background task to watch for file changes
    spawn_filewatcher(btx, file_path).await?;

    // spawns a websocket task to listen for connections
    tokio::spawn(spawn_websocket(listeners.clone()));

    while let Ok(changes) = rx.recv().await {
        let listeners = listeners.lock().unwrap();

        // why do I have to use .iter()? Why not just for item in listeners?
        for listener in listeners.iter() {
            listener.tx.send(changes.clone()).await.unwrap();
        }
    }

    Ok(())
}

#[tokio::main]
pub async fn main() {
    Builder::new()
        .format(|buf, record| {
            writeln!(
                buf,
                "{} [{}] - {}",
                Local::now().format("%Y-%m-%dT%H:%M:%S.%f"),
                record.level(),
                record.args()
            )
        })
        .filter(None, LevelFilter::Info)
        .init();

    if let Err(e) = run().await {
        log::error!("Error = {}", e);
    }
}
