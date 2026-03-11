use drift_rs::types::Context;
use std::env;

pub struct Config {
    pub rpc_url: String,
    pub ws_url: String,
    pub redis_url: String,
    pub drift_env: Context,
}

impl Config {
    pub fn from_env() -> Self {
        dotenv::dotenv().ok();
        Self {
            rpc_url: env::var("RPC_URL").expect("RPC_URL must be set"),
            ws_url: env::var("WS_URL").expect("WS_URL must be set"),
            redis_url: env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string()),
            drift_env: match env::var("DRIFT_ENV").unwrap_or_default().as_str() {
                "mainnet" => Context::MainNet,
                _ => Context::DevNet,
            },
        }
    }
}
