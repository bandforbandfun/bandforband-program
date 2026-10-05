use anchor_lang::prelude::*;

#[event]
pub struct ConfigUpdated {
    pub admin: Pubkey,
    pub settler: Pubkey,
    pub paused: bool,
    pub backing_enabled: bool,
}

#[event]
pub struct VaultOpened {
    pub owner: Pubkey,
}

#[event]
pub struct VaultClosed {
    pub owner: Pubkey,
}

#[event]
pub struct Deposited {
    pub owner: Pubkey,
    pub amount: u64,
}

#[event]
pub struct Withdrawn {
    pub owner: Pubkey,
    pub destination: Pubkey,
    pub amount: u64,
    pub fee: u64,
}

#[event]
pub struct ContestCreated {
    pub contest: Pubkey,
    pub id: [u8; 16],
    pub creator: Pubkey,
    pub stake: u64,
    pub capacity: u16,
    pub sides: u16,
    pub join_deadline: i64,
    pub duration: i64,
    pub backing_allowed: bool,
}

#[event]
pub struct Joined {
    pub contest: Pubkey,
    pub player: Pubkey,
    pub payer: Pubkey,
    pub side: u16,
    pub index: u16,
    pub stake: u64,
}

#[event]
pub struct Left {
    pub contest: Pubkey,
    pub player: Pubkey,
    pub refunded: u64,
}

#[event]
pub struct ContestLocked {
    pub contest: Pubkey,
    pub starts_at: i64,
    pub ends_at: i64,
    pub entrants: u16,
}

#[event]
pub struct ContestCancelled {
    pub contest: Pubkey,
    pub by: Pubkey,
    pub reason: u8,
}

#[event]
pub struct ContestSettled {
    pub contest: Pubkey,
    pub outcome_kind: u8,
    pub winning_mask: u64,
    pub fee: u64,
    pub backing_fee: u64,
    pub result_hash: [u8; 32],
}

#[event]
pub struct EntryClaimed {
    pub contest: Pubkey,
    pub player: Pubkey,
    pub payer: Pubkey,
    pub amount: u64,
    pub refund: bool,
}

#[event]
pub struct BackingPlaced {
    pub contest: Pubkey,
    pub backer: Pubkey,
    pub side: u16,
    pub amount: u64,
    pub total: u64,
}

#[event]
pub struct BackingClaimed {
    pub contest: Pubkey,
    pub backer: Pubkey,
    pub amount: u64,
    pub refund: bool,
}

#[event]
pub struct ContestClosed {
    pub contest: Pubkey,
    pub swept: u64,
}

#[event]
pub struct Purchased {
    pub order_id: [u8; 16],
    pub buyer: Pubkey,
    pub amount: u64,
}

#[event]
pub struct Tipped {
    pub order_id: [u8; 16],
    pub from: Pubkey,
    pub to: Pubkey,
    pub amount: u64,
    pub fee: u64,
}

#[event]
pub struct PromoCredited {
    pub order_id: [u8; 16],
    pub recipient: Pubkey,
    pub amount: u64,
}

#[event]
pub struct AccountMigrated {
    pub account: Pubkey,
    pub from_size: u32,
    pub to_size: u32,
    pub from_version: u8,
    pub to_version: u8,
}
