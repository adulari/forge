use transaction_ledger::{Account, Ledger, LedgerError, Transfer};

fn account(id: &str, balance: u64) -> Account {
    Account {
        id: id.to_owned(),
        balance,
    }
}

fn transfer(from: &str, to: &str, amount: u64) -> Transfer {
    Transfer {
        from: from.to_owned(),
        to: to.to_owned(),
        amount,
    }
}

#[test]
fn rejects_invalid_account_sets_without_reordering() {
    assert_eq!(
        Ledger::new(vec![account("", 1)]).unwrap_err(),
        LedgerError::EmptyAccountId
    );
    assert_eq!(
        Ledger::new(vec![account("a", 1), account("a", 2)]).unwrap_err(),
        LedgerError::DuplicateAccount("a".to_owned())
    );
    let ledger = Ledger::new(vec![account("z", 3), account("a", 4)]).unwrap();
    assert_eq!(ledger.snapshot(), vec![account("z", 3), account("a", 4)]);
}

#[test]
fn applies_sequentially_and_preserves_creation_order() {
    let mut ledger = Ledger::new(vec![account("c", 0), account("a", 10), account("b", 0)]).unwrap();
    let receipt = ledger
        .apply_batch(
            "chain",
            &[transfer("a", "b", 7), transfer("b", "c", 5)],
        )
        .unwrap();
    assert_eq!(receipt.applied, 2);
    assert_eq!(
        receipt.balances,
        vec![account("c", 5), account("a", 3), account("b", 2)]
    );
}

#[test]
fn rolls_back_everything_when_a_later_account_is_unknown() {
    let initial = vec![account("a", 10), account("b", 1)];
    let mut ledger = Ledger::new(initial.clone()).unwrap();
    assert_eq!(
        ledger.apply_batch(
            "unknown",
            &[transfer("a", "b", 4), transfer("b", "missing", 1)],
        ),
        Err(LedgerError::UnknownAccount {
            index: 1,
            id: "missing".to_owned(),
        })
    );
    assert_eq!(ledger.snapshot(), initial);
}

#[test]
fn rolls_back_on_later_insufficient_funds() {
    let initial = vec![account("a", 8), account("b", 0), account("c", 0)];
    let mut ledger = Ledger::new(initial.clone()).unwrap();
    assert_eq!(
        ledger.apply_batch(
            "short",
            &[transfer("a", "b", 3), transfer("a", "c", 6)],
        ),
        Err(LedgerError::InsufficientFunds {
            index: 1,
            account: "a".to_owned(),
            available: 5,
            required: 6,
        })
    );
    assert_eq!(ledger.snapshot(), initial);
}

#[test]
fn reports_destination_overflow_and_rolls_back() {
    let initial = vec![account("a", 2), account("b", u64::MAX - 1)];
    let mut ledger = Ledger::new(initial.clone()).unwrap();
    assert_eq!(
        ledger.apply_batch("overflow", &[transfer("a", "b", 2)]),
        Err(LedgerError::Overflow {
            index: 0,
            account: "b".to_owned(),
        })
    );
    assert_eq!(ledger.snapshot(), initial);
}

#[test]
fn exact_retry_returns_original_receipt_without_reapplying() {
    let mut ledger = Ledger::new(vec![account("a", 20), account("b", 0)]).unwrap();
    let batch = [transfer("a", "b", 5)];
    let original = ledger.apply_batch("once", &batch).unwrap();
    ledger
        .apply_batch("later", &[transfer("a", "b", 2)])
        .unwrap();
    let before_retry = ledger.snapshot();
    assert_eq!(ledger.apply_batch("once", &batch).unwrap(), original);
    assert_eq!(ledger.snapshot(), before_retry);
}

#[test]
fn conflicting_retry_is_rejected_without_mutation() {
    let mut ledger = Ledger::new(vec![account("a", 20), account("b", 0)]).unwrap();
    ledger
        .apply_batch("same-id", &[transfer("a", "b", 5)])
        .unwrap();
    let before = ledger.snapshot();
    assert_eq!(
        ledger.apply_batch("same-id", &[transfer("a", "b", 6)]),
        Err(LedgerError::BatchConflict {
            batch_id: "same-id".to_owned(),
        })
    );
    assert_eq!(ledger.snapshot(), before);
}

#[test]
fn validates_batch_and_transfer_values_and_accepts_empty_batches() {
    let mut ledger = Ledger::new(vec![account("a", 2), account("b", 0)]).unwrap();
    assert_eq!(ledger.apply_batch("  ", &[]), Err(LedgerError::InvalidBatchId));
    assert_eq!(
        ledger.apply_batch("zero", &[transfer("a", "b", 0)]),
        Err(LedgerError::InvalidAmount { index: 0 })
    );
    assert_eq!(
        ledger.apply_batch("self", &[transfer("a", "a", 1)]),
        Err(LedgerError::SameAccount {
            index: 0,
            id: "a".to_owned(),
        })
    );
    let empty = ledger.apply_batch("empty", &[]).unwrap();
    assert_eq!(empty.applied, 0);
    assert_eq!(ledger.apply_batch("empty", &[]).unwrap(), empty);
}
