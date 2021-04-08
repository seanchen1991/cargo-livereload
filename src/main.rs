extern crate notify;

use std::error;
use std::io::Write;
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::{Duration, Instant};

use chrono::Local;
use env_logger::Builder;
use log::LevelFilter;
use notify::{raw_watcher, RawEvent, RecursiveMode, Watcher};
use structopt::StructOpt;
use tokio::sync::broadcast;
use tokio::net::{TcpListener, TcpStream};

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

async fn spawn_filewatcher(tx: broadcast::Sender<Vec<FileChange>>, file_path: String) -> Result<(), Box<dyn error::Error>> {
    tokio::spawn(async move {
        let (watcher_tx, watcher_rx) = channel();
        // let mut watcher = raw_watcher(watcher_tx).map_err(|e| e.to_string())?;
        let mut watcher = raw_watcher(watcher_tx).expect("Error: Initializing Raw Watcher");
        // watcher.watch(file_path, RecursiveMode::Recursive).map_err(|e| e.to_string())?;
        watcher
            .watch(file_path, RecursiveMode::Recursive)
            .expect("Error: Watcher");
        log::info!("File watcher started...");

        let mut changes: Vec<FileChange> = vec!();
        let mut last_change: Option<Instant> = None;

        loop {
            match watcher_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(RawEvent {
                    path: Some(path),
                    op: Ok(op),
                    cookie: _cookie,
                }) => changes.push(FileChange::new(op, path.to_str().unwrap().to_string())),
                Ok(event) => log::error!("broken event: {:?}", event),
                Err(e) => match e {
                    RecvTimeoutError::Timeout => (),
                    RecvTimeoutError::Disconnected => {
                        log::error!("Notify disconnected");
                        break;
                    },
                }
            };

            if changes.len() > 0 {
                let should_send = match last_change {
                    Some(t) => Instant::now().duration_since(t) > Duration::from_secs(1),
                    None => {
                        last_change = Some(Instant::now());
                        false
                    },
                };
                if !should_send {
                    continue;
                }

                if let Err(e) = tx.send(changes) {
                    log::error!("Error sending changes: {:?}", e);
                }
                changes = vec!();
                last_change = None;
            }
        }
    });


    Ok(())
}

async fn run() -> Result<(), Box<dyn error::Error>> {
    let args = Cli::from_args();

    let file_path = match args.file_path {
        Some(fp) => fp.to_str().unwrap().to_string(),
        None => ".".to_string(),
    };

    let (tx, mut rx): (broadcast::Sender<Vec<FileChange>>, broadcast::Receiver<Vec<FileChange>>) = broadcast::channel(10);

    // spawns a background task to watch for file changes
    spawn_filewatcher(tx, file_path).await?;

    while let Ok(file_changes) = rx.recv().await {
        for file_change in file_changes {
            log::info!("Path: {} Operation: {:?}", file_change.path, file_change.operation);
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
