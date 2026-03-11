use lazy_static::lazy_static;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Ok = 0,
    StaleBulkAccountLoader,
    UnhealthySlotSubscriber,
    LivenessTesting,
    Restart,
}

pub struct HealthCheckConfig {
    pub check_interval_ms: u64,
    pub max_slot_staleness_ms: u64,
    pub min_slot_rate: f64,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        dotenv::dotenv().ok();
        Self {
            check_interval_ms: 2000,
            max_slot_staleness_ms: std::env::var("MAX_SLOT_STALENESS_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(5000),
            min_slot_rate: std::env::var("MIN_SLOT_RATE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.03),
        }
    }
}

pub struct HealthState {
    pub last_slot: i64,
    pub last_slot_timestamp: u128,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis()
}

lazy_static! {
    static ref GLOBAL_HEALTH_STATE: RwLock<HealthState> = RwLock::new(HealthState {
        last_slot: -1,
        last_slot_timestamp: now_ms(),
    });
    static ref STATUS: RwLock<HealthStatus> = RwLock::new(HealthStatus::Ok);
    pub static ref CONFIG: HealthCheckConfig = HealthCheckConfig::default();
}

pub fn get_health_status() -> HealthStatus {
    *STATUS.read().expect("Failed to read health status")
}

pub fn set_health_status(status: HealthStatus) {
    let mut s = STATUS.write().expect("Failed to write health status");
    *s = status;
}

/// Evaluates if the current state is healthy based on slot progression
pub fn evaluate_health(current_slot: u64) -> (bool, Option<String>) {
    let now = now_ms();
    let mut state = GLOBAL_HEALTH_STATE
        .write()
        .expect("Failed to write global health state");

    // First health check
    if state.last_slot == -1 {
        state.last_slot = current_slot as i64;
        state.last_slot_timestamp = now;
        return (true, None);
    }

    let current_status = get_health_status();
    if current_status != HealthStatus::Ok {
        return (
            false,
            Some(format!("Unhealthy state: {:?}", current_status)),
        );
    }

    let time_delta = now - state.last_slot_timestamp;
    let slot_delta = (current_slot as i64) - state.last_slot;

    // If slot has progressed, we are healthy, check rate and update state.
    if current_slot as i64 > state.last_slot {
        // Update state
        state.last_slot = current_slot as i64;
        state.last_slot_timestamp = now;

        // Check if slot update rate is too low
        let slot_rate = (slot_delta as f64 / time_delta as f64) * 1000.0; // Convert to per second
        if slot_rate < CONFIG.min_slot_rate {
            return (
                false,
                Some(format!(
                    "Slot update rate {:.2} slots/sec below minimum {}",
                    slot_rate, CONFIG.min_slot_rate
                )),
            );
        }

        return (true, None);
    }

    // If slot has NOT progressed, check for staleness.
    if time_delta > CONFIG.max_slot_staleness_ms as u128 {
        return (
            false,
            Some(format!(
                "No slot updates in {}ms (max {}ms)",
                time_delta, CONFIG.max_slot_staleness_ms
            )),
        );
    }

    // Slot has not progressed, but not stale yet. Still healthy.
    (true, None)
}
