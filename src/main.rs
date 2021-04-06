extern crate notify;

use structopt::StructOpt;

use notify::{Watcher, RecursiveMode, RawEvent, raw_watcher};
use std::sync::mpsc::channel;
use tokio::sync::mpsc;

use std::io::Write;
use chrono::Local;
use env_logger::Builder;
use log::LevelFilter;

#[derive(StructOpt)]
struct Cli {
    #[structopt(parse(from_os_str))]
    file_path: Option<std::path::PathBuf>,
}

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

struct FileChange {
    path: String,
    operation: FileChangeOperation,
}

impl From<notify::Op> for FileChangeOperation {
    fn from(source: notify::Op) -> FileChangeOperation {
        match source {
            notify::Op::CHMOD => FileChangeOperation::Chmod,
            notify::Op::CLOSE_WRITE	=> FileChangeOperation::CloseWrite,
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
        FileChange{
            path,
            operation: FileChangeOperation::from(op),
        }
    }
}

#[tokio::main]
pub async fn main() {
    Builder::new()
        .format(|buf, record| {
            writeln!(buf,
                "{} [{}] - {}",
                Local::now().format("%Y-%m-%dT%H:%M:%S.%f"),
                record.level(),
                record.args()
            )
        })
        .filter(None, LevelFilter::Info)
        .init();

    let args = Cli::from_args();

    let file_path = match args.file_path {
        Some(fp) => fp.to_str().unwrap().to_string(),
        None => ".".to_string(),
    };

    let (tx, mut rx) = mpsc::channel(32);

    tokio::spawn(async move {
        let (watcher_tx, watcher_rx) = channel();
        let mut watcher = raw_watcher(watcher_tx).unwrap();
        watcher.watch(file_path, RecursiveMode::Recursive).unwrap();
        log::info!("File watcher started...");

        loop {
            let msg = match watcher_rx.recv() {
               Ok(RawEvent{path: Some(path), op: Ok(op), cookie}) => {
                    // Ok(FileChange{path: path.to_string(), operation: 
                   format!("{:?} {:?} ({:?})", op, path, cookie)
               },
               Ok(event) => format!("broken event: {:?}", event),
               Err(e) => format!("watch error: {:?}", e),
            };
            tx.send(msg).await;
        }
    });

    while let Some(message) = rx.recv().await {
        log::info!("Got = {}", message);
    }
}
