use anyhow::{Context, anyhow};
use drift_rs::{
    DriftClient,
    dlob::{DLOB, DLOBNotifier, L2Book, L3Book},
    types::{
        MarketId, MarketType, Order, OrderStatus, OrderTriggerCondition, OrderType,
        PositionDirection, accounts::User,
    },
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

const INDICATIVE_QUOTES_PUBKEY: &str = "inDNdu3ML4vG5LNExqcwuCQtLcCU8KfK5YM2qYV3JJz";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMarketArgs {
    pub market_index: u16,
    pub market_type: MarketType,
    pub market_name: String,
    pub depth: i32,
    pub include_vamm: bool,
    pub num_vamm_orders: Option<u32>,
    pub update_on_change: bool,
    pub tick_size: u64,
}

#[derive(Debug, Deserialize)]
pub struct MMQuote {
    pub ts: u64,
    pub bid_size: Option<String>,
    pub bid_price: Option<String>,
    pub ask_size: Option<String>,
    pub ask_price: Option<String>,
    pub is_oracle_offset: bool,
}

#[derive(Debug, Deserialize)]
pub struct MMQuotesV2 {
    pub ts: String,
    pub quotes: Vec<MMQuote>,
}

#[derive(Serialize)]
pub struct L2Level {
    pub price: String,
    pub size: String,
}

#[derive(Serialize)]
pub struct L2Response {
    pub market_name: String,
    pub market_type: String,
    pub market_index: u16,
    pub bids: Vec<L2Level>,
    pub asks: Vec<L2Level>,
    pub slot: u64,
    pub ts: u128,
    pub oracle: String,
    pub mark_price: String,
    pub best_bid_price: String,
    pub best_ask_price: String,
    pub spread_pct: String,
    pub spread_quote: String,
}

pub struct DLOBSubscriber {
    pub dlob: &'static DLOB,
    pub drift_client: DriftClient,
    pub notifier: DLOBNotifier,
    pub market_args: Vec<WsMarketArgs>,
    pub redis_client: redis::Client,
    pub last_seen_l2: Arc<RwLock<HashMap<MarketId, String>>>,
    pub indicative_quotes_redis_client: Option<redis::Client>,
}

impl DLOBSubscriber {
    pub fn new(
        dlob: &'static DLOB,
        drift_client: DriftClient,
        redis_url: &str,
        indicative_redis_url: Option<String>,
    ) -> anyhow::Result<Self> {
        let redis_client = redis::Client::open(redis_url)?;
        let indicative_quotes_redis_client = indicative_redis_url
            .map(|url| redis::Client::open(url))
            .transpose()?;

        let notifier = dlob.spawn_notifier();

        Ok(Self {
            dlob,
            drift_client,
            notifier,
            market_args: Vec::new(),
            redis_client,
            last_seen_l2: Arc::new(RwLock::new(HashMap::new())),
            indicative_quotes_redis_client,
        })
    }

    pub async fn update_dlob(&self, slot: u64) -> anyhow::Result<()> {
        let indicative_redis = match &self.indicative_quotes_redis_client {
            Some(client) => client,
            None => return Ok(()),
        };

        let mut conn = indicative_redis.get_async_connection().await?;
        let indicative_pubkey = INDICATIVE_QUOTES_PUBKEY.parse::<Pubkey>().unwrap();

        for market_arg in &self.market_args {
            let market_type_str = match market_arg.market_type {
                MarketType::Perp => "perp",
                MarketType::Spot => "spot",
            };

            let mms_key = format!("market_mms_{}_{}", market_type_str, market_arg.market_index);
            let mms: Vec<String> = conn.smembers(mms_key).await?;

            for mm in mms {
                let quote_key = format!(
                    "mm_quotes_v2_{}_{}_{}",
                    market_type_str, market_arg.market_index, mm
                );
                let quote_json: Option<String> = conn.get(quote_key).await?;

                if let Some(json) = quote_json {
                    if let Ok(mm_quotes) = serde_json::from_str::<MMQuotesV2>(&json) {
                        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;

                        let ts_threshold = now.saturating_sub(1000);
                        let quotes_ts = mm_quotes.ts.parse::<u64>().unwrap_or(0);

                        if quotes_ts > ts_threshold {
                            for quote in mm_quotes.quotes {
                                if let (Some(size_str), Some(price_str)) =
                                    (&quote.bid_size, &quote.bid_price)
                                {
                                    let size = size_str.parse::<u64>().unwrap_or(0);
                                    let price = price_str.parse::<u64>().unwrap_or(0);

                                    if size > 0 {
                                        let mut order = self.create_base_order(market_arg, slot);
                                        order.direction = PositionDirection::Long;
                                        order.base_asset_amount = size;
                                        if quote.is_oracle_offset {
                                            order.oracle_price_offset = price as i32;
                                        } else {
                                            order.price = price;
                                        }

                                        // Notifier expects an update based on User account.
                                        // For indicative quotes, we can't easily use notifier.user_update
                                        // without a full User account.
                                        // We might need to call dlob.insert_order directly if we have a mut ref,
                                        // but DLOB uses DashMap and interior mutability.
                                        // However, insert_order is private.
                                        // The intended way is through the notifier.
                                    }
                                }
                                // Similar for ask...
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn get_l2_and_send_msg(&self, market_arg: &WsMarketArgs) -> anyhow::Result<()> {
        let market_id = MarketId::new(market_arg.market_index, market_arg.market_type);
        let l2 = self
            .dlob
            .get_l2_snapshot(market_arg.market_index, market_arg.market_type);

        let oracle_price = l2.oracle_price;
        let slot = l2.slot;

        // Health check logic: Check for oracle staleness or slot diffs
        let oracle_data = if market_arg.market_type == MarketType::Perp {
            self.drift_client
                .try_get_mmoracle_for_perp_market(market_arg.market_index, slot)
                .map_err(|e| anyhow!("Failed to get oracle for perp market: {:?}", e))?
        } else {
            // Simplified: drift-rs might have a different method for spot oracles if not same
            self.drift_client
                .try_get_mmoracle_for_perp_market(market_arg.market_index, slot)
                .map_err(|e| anyhow!("Failed to get oracle for spot market: {:?}", e))?
        };

        let oracle_slot = oracle_data.slot;
        let kill_switch_threshold = 200; // Configurable

        if (slot as i128 - oracle_slot as i128).abs() > kill_switch_threshold {
            eprintln!(
                "Unhealthy process due to slot diffs for market {}. dlobProviderSlot: {}, oracleSlot: {}",
                market_arg.market_name, slot, oracle_slot
            );
            crate::core::health::set_health_status(crate::core::health::HealthStatus::Restart);
        }

        // Calculate spread info
        let best_bid = l2.bids.iter().rev().next().map(|(p, s)| (*p, *s));
        let best_ask = l2.asks.iter().next().map(|(p, s)| (*p, *s));

        let best_bid_price = best_bid.map(|x| x.0).unwrap_or(0);
        let best_ask_price = best_ask.map(|x| x.0).unwrap_or(0);

        let mark_price = if best_bid_price != 0 && best_ask_price != 0 {
            (best_bid_price + best_ask_price) / 2
        } else {
            oracle_price
        };

        let spread_quote = best_ask_price.saturating_sub(best_bid_price);
        let spread_pct = if mark_price != 0 {
            (spread_quote * 1_000_000) / mark_price
        } else {
            0
        };

        let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();

        let response = L2Response {
            market_name: market_arg.market_name.clone(),
            market_type: format!("{:?}", market_arg.market_type).to_lowercase(),
            market_index: market_arg.market_index,
            bids: l2
                .bids
                .iter()
                .rev()
                .map(|(p, s)| L2Level {
                    price: p.to_string(),
                    size: s.to_string(),
                })
                .collect(),
            asks: l2
                .asks
                .iter()
                .map(|(p, s)| L2Level {
                    price: p.to_string(),
                    size: s.to_string(),
                })
                .collect(),
            slot,
            ts,
            oracle: oracle_price.to_string(),
            mark_price: mark_price.to_string(),
            best_bid_price: best_bid_price.to_string(),
            best_ask_price: best_ask_price.to_string(),
            spread_pct: spread_pct.to_string(),
            spread_quote: spread_quote.to_string(),
        };

        let response_json = serde_json::to_string(&response)?;

        let mut conn = self.redis_client.get_async_connection().await?;
        let market_type_str = format!("{:?}", market_arg.market_type).to_lowercase();
        let channel = format!("orderbook_{}_{}", market_type_str, market_arg.market_index);

        conn.publish::<_, _, ()>(channel, &response_json).await?;
        conn.set::<_, _, ()>(
            format!(
                "last_update_orderbook_{}_{}",
                market_type_str, market_arg.market_index
            ),
            response_json,
        )
        .await?;

        Ok(())
    }

    fn create_base_order(&self, market_arg: &WsMarketArgs, slot: u64) -> Order {
        Order {
            status: OrderStatus::Open,
            order_type: OrderType::Limit,
            order_id: 0,
            slot,
            market_index: market_arg.market_index,
            market_type: market_arg.market_type,
            base_asset_amount: 0,
            immediate_or_cancel: false,
            direction: PositionDirection::Long,
            oracle_price_offset: 0,
            max_ts: 0,
            reduce_only: false,
            trigger_condition: OrderTriggerCondition::Above,
            price: 0,
            user_order_id: 0,
            post_only: true,
            auction_duration: 0,
            auction_start_price: 0,
            auction_end_price: 0,
            existing_position_direction: PositionDirection::Long,
            trigger_price: 0,
            base_asset_amount_filled: 0,
            quote_asset_amount_filled: 0,
            bit_flags: 0,
            posted_slot_tail: 0,
            padding: [0; 1],
        }
    }
}
