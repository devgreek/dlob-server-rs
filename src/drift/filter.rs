use drift_rs::{
    account_map::AccountMap, dlob::DLOBNotifier, grpc::AccountUpdate, types::accounts::User,
};
use solana_sdk::pubkey::Pubkey;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct OrderSubscriberFiltered {
    pub ignore_list: HashSet<Pubkey>,
    pub most_recent_slot: AtomicU64,
}

impl OrderSubscriberFiltered {
    pub fn new(ignore_list: HashSet<Pubkey>) -> Self {
        Self {
            ignore_list,
            most_recent_slot: AtomicU64::new(0),
        }
    }

    /// Returns a filtered account update handler for gRPC/WebSocket subscriptions
    pub fn account_update_handler<'a>(
        &'a self,
        account_map: &'a AccountMap,
        notifier: DLOBNotifier,
    ) -> impl Fn(&AccountUpdate) + Send + Sync + 'a {
        move |update| {
            // Update most recent slot
            let current_max = self.most_recent_slot.load(Ordering::Relaxed);
            if update.slot > current_max {
                self.most_recent_slot.store(update.slot, Ordering::Relaxed);
            }

            // Filter by ignore list
            if self.ignore_list.contains(&update.pubkey) {
                return;
            }

            // Deserialize and notify DLOB
            let new_user: &User = drift_rs::utils::deser_zero_copy(update.data);
            let old_user = account_map
                .account_data_and_slot::<User>(&update.pubkey)
                .map(|x| x.data);

            notifier.user_update(update.pubkey, old_user.as_ref(), new_user, update.slot);
        }
    }

    pub fn get_most_recent_slot(&self) -> u64 {
        self.most_recent_slot.load(Ordering::Relaxed)
    }
}
