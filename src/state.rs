//! Shared AppState (Arc-wrapped managers)
use crate::drift::subscriber::DLOBSubscriber;
use drift_rs::DriftClient;
use std::sync::Arc;

pub struct AppState {
    pub drift_client: DriftClient,
    pub dlob_subscriber: Arc<DLOBSubscriber>,
}

pub type SharedState = Arc<AppState>;
