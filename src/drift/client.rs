use drift_rs::{DriftClient, Wallet, types::Context};
use solana_rpc_client::nonblocking::rpc_client::RpcClient;

pub async fn create_drift_client(rpc_url: &str, context: Context) -> anyhow::Result<DriftClient> {
    let rpc_client = RpcClient::new(rpc_url.to_string());
    // Using a dummy wallet for now as the server is mainly a reader/subscriber
    let wallet = Wallet::from_seed_bs58(
        "58D667qSxh8XQ4F4kE8ZqJcK8ZqJcK8ZqJcK8ZqJcK8ZqJcK8ZqJcK8ZqJcK8ZqJcK8ZqJcK8ZqJcK8Z",
    );
    let client = DriftClient::new(context, rpc_client, wallet)?;
    Ok(client)
}
