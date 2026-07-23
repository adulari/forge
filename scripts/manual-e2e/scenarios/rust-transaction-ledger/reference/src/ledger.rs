use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub balance: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub from: String,
    pub to: String,
    pub amount: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub batch_id: String,
    pub applied: usize,
    pub balances: Vec<Account>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedgerError {
    EmptyAccountId,
    DuplicateAccount(String),
    InvalidBatchId,
    InvalidAmount {
        index: usize,
    },
    SameAccount {
        index: usize,
        id: String,
    },
    UnknownAccount {
        index: usize,
        id: String,
    },
    InsufficientFunds {
        index: usize,
        account: String,
        available: u64,
        required: u64,
    },
    Overflow {
        index: usize,
        account: String,
    },
    BatchConflict {
        batch_id: String,
    },
}

#[derive(Debug)]
pub struct Ledger {
    accounts: Vec<Account>,
    index: HashMap<String, usize>,
    seen: HashMap<String, (Vec<Transfer>, Receipt)>,
}

impl Ledger {
    pub fn new(accounts: Vec<Account>) -> Result<Self, LedgerError> {
        let mut seen_ids = HashSet::new();
        for account in &accounts {
            if account.id.is_empty() {
                return Err(LedgerError::EmptyAccountId);
            }
            if seen_ids.contains(&account.id) {
                return Err(LedgerError::DuplicateAccount(account.id.clone()));
            }
            seen_ids.insert(&account.id);
        }

        let index = accounts
            .iter()
            .enumerate()
            .map(|(position, account)| (account.id.clone(), position))
            .collect();
        Ok(Self {
            accounts,
            index,
            seen: HashMap::new(),
        })
    }

    pub fn snapshot(&self) -> Vec<Account> {
        self.accounts.clone()
    }

    pub fn apply_batch(
        &mut self,
        batch_id: &str,
        transfers: &[Transfer],
    ) -> Result<Receipt, LedgerError> {
        if batch_id.trim().is_empty() {
            return Err(LedgerError::InvalidBatchId);
        }

        // Check if this batch_id was seen before
        if let Some((original_transfers, original_receipt)) = self.seen.get(batch_id) {
            // If the transfers are identical, return the original receipt
            if original_transfers == transfers {
                return Ok(original_receipt.clone());
            } else {
                return Err(LedgerError::BatchConflict {
                    batch_id: batch_id.to_owned(),
                });
            }
        }

        // Validate all transfers before applying any changes
        for (position, transfer) in transfers.iter().enumerate() {
            if transfer.amount == 0 {
                return Err(LedgerError::InvalidAmount { index: position });
            }
            if transfer.from == transfer.to {
                return Err(LedgerError::SameAccount {
                    index: position,
                    id: transfer.from.clone(),
                });
            }
            if !self.index.contains_key(&transfer.from) {
                return Err(LedgerError::UnknownAccount {
                    index: position,
                    id: transfer.from.clone(),
                });
            }
            if !self.index.contains_key(&transfer.to) {
                return Err(LedgerError::UnknownAccount {
                    index: position,
                    id: transfer.to.clone(),
                });
            }
        }

        // Compute changes in a temporary copy to ensure atomicity
        let mut new_accounts = self.accounts.clone();
        for (position, transfer) in transfers.iter().enumerate() {
            let from_idx = self.index[&transfer.from];
            let to_idx = self.index[&transfer.to];
            let available = new_accounts[from_idx].balance;

            if available < transfer.amount {
                return Err(LedgerError::InsufficientFunds {
                    index: position,
                    account: transfer.from.clone(),
                    available,
                    required: transfer.amount,
                });
            }

            // Check for overflow in destination
            if new_accounts[to_idx]
                .balance
                .checked_add(transfer.amount)
                .is_none()
            {
                return Err(LedgerError::Overflow {
                    index: position,
                    account: transfer.to.clone(),
                });
            }

            new_accounts[from_idx].balance -= transfer.amount;
            new_accounts[to_idx].balance += transfer.amount;
        }

        // All checks passed, apply changes
        self.accounts = new_accounts;
        let receipt = Receipt {
            batch_id: batch_id.to_owned(),
            applied: transfers.len(),
            balances: self.snapshot(),
        };

        // Store the batch for idempotency checks
        self.seen
            .insert(batch_id.to_owned(), (transfers.to_vec(), receipt.clone()));
        Ok(receipt)
    }
}
