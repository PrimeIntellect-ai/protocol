use log::{debug, LevelFilter};
use tracing_subscriber::filter::EnvFilter as TracingEnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use url::Url;

use crate::cli::command::Commands;
use crate::cli::Cli;
use anyhow::Result;
use std::time::{SystemTime, UNIX_EPOCH};
use time::macros::format_description;
use tracing_subscriber::fmt::time::FormatTime;

struct SimpleTimeFormatter;

impl FormatTime for SimpleTimeFormatter {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        // Get current time
        let now = SystemTime::now();
        let timestamp = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

        // Convert to time::OffsetDateTime
        let datetime = time::OffsetDateTime::from_unix_timestamp(timestamp as i64)
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);

        // Format as hh:mm:ss
        let format = format_description!("[hour]:[minute]:[second]");
        let formatted = datetime
            .format(format)
            .unwrap_or_else(|_| String::from("??:??:??"));

        write!(w, "{}", formatted)
    }
}

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
        .add_directive("reqwest=warn".parse()?)
        .add_directive("hyper=warn".parse()?)
        .add_directive("hyper_util=warn".parse()?)
        .add_directive("bollard=warn".parse()?)
        .add_directive("alloy=warn".parse()?)
        .add_directive("iroh=error".parse()?)
        .add_directive("iroh_net=error".parse()?)
        .add_directive("iroh_quinn=error".parse()?)
        .add_directive("iroh_base=error".parse()?)
        .add_directive("quinn=error".parse()?)
        .add_directive("quinn_proto=error".parse()?)
        .add_directive("tracing::span=warn".parse()?);

    let fmt_layer = fmt::layer()
        .with_target(false)
        .with_level(true)
        .with_ansi(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(false)
        .with_line_number(false)
        .with_timer(SimpleTimeFormatter)
        .compact();

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    if let Some(loki_url_str) = loki_url {
        let loki_url_parsed = Url::parse(&loki_url_str)?;

        let ip: String = match external_ip {
            Some(Some(ip_addr)) => ip_addr.to_string(),
            _ => "Unknown".to_string(),
        };

        let (loki_layer, task) = tracing_loki::builder()
            .label("app", "prime-worker")?
            .label("version", env!("CARGO_PKG_VERSION"))?
            .label("external_ip", ip)?
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
