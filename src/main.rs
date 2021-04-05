use structopt::StructOpt;

use std::io::Write;
use tokio::sync::mpsc;

use log::LevelFilter;
use chrono::Local;
use env_logger::Builder;
use notify::{Watcher, RecommendedWatcher, RecursiveMode, Result};

#[derive(StructOpt)]
struct Cli {
    #[structopt(parse(from_os_str))]
    file_path: Option<std::path::PathBuf>,
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
        let mut watcher: RecommendedWatcher = Watcher::new_immediate(|res| {
            match res {
                Ok(event) => println!("event: {:?}", event),
                Err(e) => println!("watch error: {:?}", e),
            }
        })?;

        watcher.watch(".", RecursiveMode::Recursive)?;
    })

    Ok(())
}
