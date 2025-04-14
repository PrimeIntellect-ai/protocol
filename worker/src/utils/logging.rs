use log::{debug, LevelFilter};
use tracing_subscriber::filter::EnvFilter as TracingEnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use url::Url;

use crate::cli::command::Commands;
use crate::cli::Cli;

pub fn setup_logging(cli: Option<&Cli>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Default log level
    let mut log_level = LevelFilter::Info;
    let mut loki_url: Option<String> = None;
    let mut external_ip = None;
    let mut compute_pool = None;
    let mut port = None;

    // Extract command-specific parameters if CLI is provided
    if let Some(cli) = cli {
        if let Commands::Run {
            external_ip: cmd_external_ip,
            port: cmd_port,
            compute_pool_id: cmd_compute_pool_id,
            loki_url: cmd_loki_url,
            log_level: cmd_log_level,
            ..
        } = &cli.command
        {
            // TODO: level parsing
            compute_pool = Some(*cmd_compute_pool_id);
            external_ip = Some(cmd_external_ip.clone());
            port = Some(*cmd_port);
            match cmd_loki_url {
                Some(url) => loki_url = Some(url.clone()),
                None => loki_url = None,
            }
            match cmd_log_level {
                Some(level) => {
                    log_level = level.parse()?;
                }
                None => log_level = LevelFilter::Info,
            }
        }
    }

    let env_filter = TracingEnvFilter::from_default_env()
        .add_directive(format!("{}", log_level).parse()?)
        .add_directive("reqwest=warn".parse()?);

    let fmt_layer = fmt::layer()
        .with_target(true)
        .with_level(true)
        .with_ansi(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(false)
        .with_line_number(false)
        .compact();

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    if let Some(loki_url_str) = loki_url {
        let loki_url_parsed = Url::parse(&loki_url_str)?;

        let (loki_layer, task) = tracing_loki::builder()
            .label("app", "prime-worker")?
            .label("version", env!("CARGO_PKG_VERSION"))?
            .label("external_ip", external_ip.unwrap_or_default())?
            .label("compute_pool", compute_pool.unwrap_or_default().to_string())?
            .label("port", port.unwrap_or_default().to_string())?
            .build_url(loki_url_parsed)?;

        tokio::spawn(task);
        registry.with(loki_layer).init();
        debug!("Logging to console and Loki at {}", loki_url_str);
    } else {
        registry.init();
    }

    Ok(())
}
