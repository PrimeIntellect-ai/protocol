use clap::Parser;
use tokio::signal::unix::{signal, SignalKind};
use tracing::Level;
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt};

#[derive(Parser)]
struct Config {
    /// Hex-encoded libp2p private key
    #[clap(long)]
    libp2p_private_key: String,

    /// Libp2p port
    #[clap(long, default_value = "4005")]
    libp2p_port: u16,

    /// Log level
    #[arg(short = 'l', long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() {
    let config = Config::parse();

    let log_level = match config.log_level.as_str() {
        "error" => Level::ERROR,
        "warn" => Level::WARN,
        "info" => Level::INFO,
        "debug" => Level::DEBUG,
        _ => {
            eprintln!("invalid log level: {}", config.log_level);
            std::process::exit(1);
        }
    };

    let env_filter =
        tracing_subscriber::filter::EnvFilter::from_default_env().add_directive(log_level.into());

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(env_filter)
        .init();

    let cancellation_token = tokio_util::sync::CancellationToken::new();

    let mut bytes = hex::decode(config.libp2p_private_key.trim()).unwrap_or_else(|_| {
        eprintln!("invalid hex-encoded libp2p private key");
        std::process::exit(1);
    });
    let keypair = p2p::Keypair::ed25519_from_bytes(&mut bytes).unwrap_or_else(|e| {
        eprintln!("failed to create ed25519 keypair from provided private key: {e}");
        std::process::exit(1);
    });

    let node = match p2p::NodeBuilder::new()
        .with_keypair(keypair)
        .with_port(config.libp2p_port)
        .with_cancellation_token(cancellation_token.clone())
        .try_build()
    {
        Ok(res) => res.0,
        Err(e) => {
            eprintln!("failed to create p2p node: {e}");
            std::process::exit(1);
        }
    };

    tokio::spawn({
        let cancellation_token = cancellation_token.clone();
        async move {
            let mut sigint = signal(SignalKind::interrupt()).expect(
                "setting a SIGINT listener should always work on unix; is this running on unix?",
            );
            let mut sigterm = signal(SignalKind::terminate()).expect(
                "setting a SIGTERM listener should always work on unix; is this running on unix?",
            );
            loop {
                tokio::select! {
                    _ = sigint.recv() => {
                        tracing::info!("received SIGINT");
                        cancellation_token.cancel();
                    }
                    _ = sigterm.recv() => {
                        tracing::info!("received SIGTERM");
                        cancellation_token.cancel();
                    }
                }
            }
        }
    });

    tokio::task::spawn(node.run());

    tracing::info!("Bootnode started with libp2p port {}", config.libp2p_port);

    cancellation_token.cancelled().await;
}
