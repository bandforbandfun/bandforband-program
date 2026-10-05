use anchor_lang::prelude::*;

use crate::constants::*;
use crate::math::Tier;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq, InitSpace)]
pub struct FeeTier {
    pub min_stake: u64,
    pub bps: u16,
}

impl From<FeeTier> for Tier {
    fn from(value: FeeTier) -> Self {
        Tier { min_stake: value.min_stake, bps: value.bps }
    }
}

#[account]
#[derive(InitSpace)]
pub struct Config {
    /// Squads multisig vault in production.
    pub admin: Pubkey,
    pub pending_admin: Pubkey,
    /// Backend key allowed to lock, settle and credit promos. It can never move funds to
    /// anything other than entrants' vaults and the treasury.
    pub settler: Pubkey,
    pub treasury_token: Pubkey,
    pub usdc_mint: Pubkey,
    pub fee_tiers: [FeeTier; MAX_FEE_TIERS],
    pub fee_tier_count: u8,
    pub backing_fee_bps: u16,
    pub backing_enabled: bool,
    pub backing_min: u64,
    pub backing_max_per_user: u64,
    pub preparation_secs: i64,
    pub refund_timeout_secs: i64,
    pub withdraw_fee_cap: u64,
    pub max_promo_per_tx: u64,
    pub paused: bool,
    pub bump: u8,
    pub rewards_bump: u8,
    pub version: u8,
    pub reserved: [u8; RESERVED_LONG],
}

impl Config {
    pub fn tiers(&self) -> Vec<Tier> {
        self.fee_tiers[..self.fee_tier_count as usize].iter().map(|t| (*t).into()).collect()
    }
}

#[account]
#[derive(InitSpace)]
pub struct UserVault {
    pub owner: Pubkey,
    pub total_deposited: u64,
    pub total_withdrawn: u64,
    pub created_at: i64,
    pub bump: u8,
    pub token_bump: u8,
    pub version: u8,
    /// Gets the rent back when the vault is closed. Unset on vaults opened before it was
    /// recorded: those can't be closed.
    pub rent_payer: Pubkey,
    /// `RESERVED_LONG` minus `rent_payer`.
    pub reserved: [u8; 32],
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum ContestKind {
    /// Two sides of `seats_per_side` (1v1, 2v2, 3v3).
    Duel,
    /// Free-for-all: every seat is its own side.
    Lobby,
    /// One pool of entrants, paid by placement.
    Tournament,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum ContestStatus {
    Open,
    /// Full (or started by the settler). PREPARING while `now < starts_at`, then LIVE.
    Locked,
    Settled,
    /// Draw: everyone refunded, no fee.
    Refunded,
    /// Cancelled before start, by timeout or by the settler: everyone refunded.
    Cancelled,
}

impl ContestStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, ContestStatus::Settled | ContestStatus::Refunded | ContestStatus::Cancelled)
    }

    pub fn refunds(&self) -> bool {
        matches!(self, ContestStatus::Refunded | ContestStatus::Cancelled)
    }
}

pub const OUTCOME_NONE: u8 = 0;
pub const OUTCOME_SIDES: u8 = 1;
pub const OUTCOME_PLACEMENTS: u8 = 2;
pub const OUTCOME_DRAW: u8 = 3;
/// Placements too many for the contest account: they live in the contest's `PlacementBook`.
pub const OUTCOME_BOOK: u8 = 4;

#[account]
#[derive(InitSpace)]
pub struct Contest {
    pub id: [u8; 16],
    pub creator: Pubkey,
    /// Receives the rent back when entries/backings/contest accounts are closed.
    pub rent_payer: Pubkey,
    pub kind: ContestKind,
    pub status: ContestStatus,
    pub stake: u64,
    pub capacity: u16,
    pub sides: u16,
    pub seats_per_side: u16,
    pub min_entrants: u16,
    pub entrants: u16,
    pub next_index: u16,
    pub side_counts: [u16; MAX_SIDES],
    pub pot: u64,
    pub join_deadline: i64,
    pub duration: i64,
    pub filled_at: i64,
    pub starts_at: i64,
    pub ends_at: i64,
    pub backing_allowed: bool,
    pub backing_pools: [u64; MAX_SIDES],
    pub backing_total: u64,
    pub outcome_kind: u8,
    pub winning_mask: u64,
    pub per_seat_payout: u64,
    pub net_pot: u64,
    pub placement_entries: [u16; MAX_PLACES],
    pub placement_bps: [u16; MAX_PLACES],
    pub placement_count: u8,
    pub backing_refund_all: bool,
    pub backing_winning_pool: u64,
    pub backing_net_losing: u64,
    pub fee_bps: u16,
    pub fee_paid: u64,
    pub result_hash: [u8; 32],
    pub open_entries: u16,
    pub open_backings: u32,
    pub bump: u8,
    pub token_bump: u8,
    pub version: u8,
    pub reserved: [u8; RESERVED_LONG],
}

impl Contest {
    pub fn is_winning_side(&self, side: u16) -> bool {
        side < 64 && self.winning_mask & (1u64 << side) != 0
    }

    pub fn placement_of(&self, index: u16) -> Option<usize> {
        self.placement_entries[..self.placement_count as usize].iter().position(|e| *e == index)
    }
}

#[account]
#[derive(InitSpace)]
pub struct Entry {
    pub contest: Pubkey,
    pub player: Pubkey,
    /// Vault owner that paid the stake and receives the payout ("covering" teammates).
    pub payer: Pubkey,
    pub rent_payer: Pubkey,
    pub side: u16,
    pub index: u16,
    pub stake: u64,
    pub bump: u8,
    pub version: u8,
    pub reserved: [u8; RESERVED_SHORT],
}

#[account]
#[derive(InitSpace)]
pub struct Backing {
    pub contest: Pubkey,
    pub backer: Pubkey,
    pub rent_payer: Pubkey,
    pub side: u16,
    pub amount: u64,
    pub bump: u8,
    pub version: u8,
    pub reserved: [u8; RESERVED_SHORT],
}

/// Paid places of a tournament settled with `settle_book`: `entries[i]` (entry index) takes
/// `bps[i]` of the net pot. Lives until the contest is closed.
#[account]
#[derive(InitSpace)]
pub struct PlacementBook {
    pub contest: Pubkey,
    pub count: u16,
    pub entries: [u16; MAX_BOOK_PLACES],
    pub bps: [u16; MAX_BOOK_PLACES],
    pub bump: u8,
    pub version: u8,
    pub reserved: [u8; RESERVED_SHORT],
}

impl PlacementBook {
    pub fn payout(&self, net_pot: u64, index: u16) -> Option<u64> {
        let count = self.count as usize;
        match self.entries[..count].iter().position(|e| *e == index) {
            Some(place) => crate::math::placement_payout(net_pot, &self.bps[..count], place),
            None => Some(0),
        }
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum OrderKind {
    Purchase,
    Tip,
    Promo,
}

/// Idempotency record: an order id can only ever be executed once.
#[account]
#[derive(InitSpace)]
pub struct Order {
    pub id: [u8; 16],
    pub kind: OrderKind,
    pub from: Pubkey,
    pub to: Pubkey,
    pub amount: u64,
    pub created_at: i64,
    pub rent_payer: Pubkey,
    pub bump: u8,
    pub version: u8,
    pub reserved: [u8; RESERVED_SHORT],
}

/// Every program account carries a layout `version`, which `migrate_account` brings up to
/// `ACCOUNT_VERSION`.
pub trait Versioned {
    fn version(&self) -> u8;
    fn set_version(&mut self, version: u8);
}

macro_rules! versioned {
    ($($t:ty),*) => {$(
        impl Versioned for $t {
            fn version(&self) -> u8 {
                self.version
            }
            fn set_version(&mut self, version: u8) {
                self.version = version;
            }
        }
    )*};
}

versioned!(Config, UserVault, Contest, Entry, Backing, Order, PlacementBook);
