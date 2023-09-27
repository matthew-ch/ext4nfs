use tokio;
use ext4nfs::Ext4FS;
use nfsserve::tcp::{self, NFSTcp};
use clap::Parser;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value_t = 11111)]
    port: u16,

    #[arg(required = true, help = "Device file path")]
    path: String,
}

fn main() {
    let args = Args::parse();
    let my_fs = Ext4FS::new_with_path(&args.path);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .with_writer(std::io::stderr)
                .init();
            let listener = tcp::NFSTcpListener::bind(&format!("localhost:{}", args.port), my_fs)
                .await
                .unwrap();
            listener.handle_forever().await.unwrap()
        });
}
