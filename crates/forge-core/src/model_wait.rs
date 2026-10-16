//! Queueing behind the only usable model when another session is serving it.

/// How long a turn whose only usable model is busy waits for it before failing as unavailable.
const RESERVATION_WAIT: std::time::Duration = std::time::Duration::from_secs(300);

/// Reserve `model` for an auto-routed completion (explicit pins bypass the scheduler). When it is
/// the last candidate and busy, queue behind it rather than failing the turn.
pub(crate) async fn reserve(
    store: &forge_store::Store,
    model: &str,
    explicit_pin: bool,
    last_candidate: bool,
) -> Option<forge_store::ModelReservation> {
    if explicit_pin {
        return None;
    }
    match store.try_reserve_model(model) {
        Some(reservation) => Some(reservation),
        None if last_candidate => wait_for_reservation(store, model).await,
        None => None,
    }
}

async fn wait_for_reservation(
    store: &forge_store::Store,
    model: &str,
) -> Option<forge_store::ModelReservation> {
    wait_for_reservation_up_to(store, model, RESERVATION_WAIT).await
}

async fn wait_for_reservation_up_to(
    store: &forge_store::Store,
    model: &str,
    max_wait: std::time::Duration,
) -> Option<forge_store::ModelReservation> {
    let deadline = std::time::Instant::now() + max_wait;
    loop {
        if let Some(reservation) = store.try_reserve_model(model) {
            return Some(reservation);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_busy_only_model_is_waited_for_then_reserved() {
        let store = forge_store::Store::open_in_memory().unwrap();
        let held = store.try_reserve_model("m").unwrap();
        let waiter = wait_for_reservation_up_to(&store, "m", std::time::Duration::from_secs(5));
        let release = async {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            drop(held);
        };
        let (got, ()) = tokio::join!(waiter, release);
        assert!(got.is_some());
        assert!(
            wait_for_reservation_up_to(&store, "m", std::time::Duration::from_millis(250))
                .await
                .is_none(),
            "a still-held model times out"
        );
    }
}
