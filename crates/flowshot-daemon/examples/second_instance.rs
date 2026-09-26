//! QA harness (todo 32 live acceptance): behaves as the SECOND `flowshot`
//! process - runs the production client handshake against the session bus
//! and exits 0 when the argv reached a running daemon. This is the exact
//! composition todo 35's CLI dispatch will call.
//!
//! Usage: `second_instance <argv tokens...>` (e.g. `second_instance
//! capture --full`) while `flowshot-daemon` is running.

#![forbid(unsafe_code)]

use flowshot_daemon::bus::SERVICE;
use flowshot_daemon::instance::{Acquisition, acquire_or_forward};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let argv: Vec<String> = std::env::args().skip(1).collect();
    match acquire_or_forward(None, SERVICE, &argv).await? {
        Acquisition::Forwarded => {
            println!("forwarded {argv:?} to the running daemon; exiting 0");
            Ok(())
        }
        Acquisition::Owner(connection) => {
            let _ = connection.close().await;
            anyhow::bail!(
                "no daemon was running - this process won the bus name instead of forwarding; \
                 start flowshot-daemon first"
            );
        }
    }
}
