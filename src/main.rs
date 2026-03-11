mod api;
mod athena;
mod config;
mod core;
mod drift;
mod publishers;
mod state;
mod utils;

use crate::config::Config;
use crate::drift::client::create_drift_client;
use crate::drift::subscriber::DLOBSubscriber;
use crate::state::AppState;
use drift_rs::dlob::builder::DLOBBuilder;
use drift_rs::types::MarketId;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = Config::from_env();

    // 1. Initialize Drift Client
    let drift_client = create_drift_client(&config.rpc_url, config.drift_env).await?;

    // 2. Initialize DLOB
    // We need an AccountMap or just start fresh.
    // For a server, we usually want to sync all users.
    println!("Syncing user accounts...");
    drift_client.sync_user_accounts(vec![]).await?;

    let account_map = drift_client.backend().account_map();
    let builder = DLOBBuilder::new(account_map);
    let dlob = builder.dlob(); // This is a 'static DLOB

    // 3. Initialize DLOB Subscriber
    let dlob_subscriber = Arc::new(DLOBSubscriber::new(
        dlob,
        drift_client.clone(),
        &config.redis_url,
        None, // indicative quotes redis url
    )?);

    // 4. Start gRPC/Subscription for DLOB updates
    // For this example, we'll just show the structure of the main loop
    let markets = vec![MarketId::perp(0)]; // Example market

    let dlob_handler = dlob_subscriber.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(100));
        loop {
            interval.tick().await;
            for market_arg in &dlob_handler.market_args {
                if let Err(e) = dlob_handler.get_l2_and_send_msg(market_arg).await {
                    eprintln!("Error sending L2: {:?}", e);
                }
            }
        }
    });

    println!("DLOB Server running...");

    // Keep alive
    tokio::signal::ctrl_c().await?;

    Ok(())
}
