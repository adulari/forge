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
    InvalidAmount { index: usize },
    SameAccount { index: usize, id: String },
    UnknownAccount { index: usize, id: String },
    InsufficientFunds {
        index: usize,
        account: String,
        available: u64,
        required: u64,
    },
    Overflow { index: usize, account: String },
    BatchConflict { batch_id: String },
}

#[derive(Debug)]
pub struct Ledger {
    accounts: Vec<Account>,
    index: HashMap<String, usize>,
    seen: HashSet<String>,
}

impl Ledger {
    pub fn new(accounts: Vec<Account>) -> Result<Self, LedgerError> {
        let index = accounts
            .iter()
            .enumerate()
            .map(|(position, account)| (account.id.clone(), position))
            .collect();
        Ok(Self {
            accounts,
            index,
            seen: HashSet::new(),
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
        if self.seen.contains(batch_id) {
            return Ok(Receipt {
                batch_id: batch_id.to_owned(),
                applied: transfers.len(),
                balances: self.snapshot(),
            });
        }

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
            let Some(&from) = self.index.get(&transfer.from) else {
                return Err(LedgerError::UnknownAccount {
                    index: position,
                    id: transfer.from.clone(),
                });
            };
            let Some(&to) = self.index.get(&transfer.to) else {
                return Err(LedgerError::UnknownAccount {
                    index: position,
                    id: transfer.to.clone(),
                });
            };
            let available = self.accounts[from].balance;
            if available < transfer.amount {
                return Err(LedgerError::InsufficientFunds {
                    index: position,
                    account: transfer.from.clone(),
                    available,
                    required: transfer.amount,
                });
            }
            self.accounts[from].balance -= transfer.amount;
            self.accounts[to].balance = self.accounts[to].balance.saturating_add(transfer.amount);
        }

        self.seen.insert(batch_id.to_owned());
        Ok(Receipt {
            batch_id: batch_id.to_owned(),
            applied: transfers.len(),
            balances: self.snapshot(),
        })
    }
}
