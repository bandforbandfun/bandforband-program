//! bandforband.fun escrow program.
//!
//! Custody model: every user balance lives in a per-user vault PDA that only its owner can
//! withdraw from. Contest stakes and backings move into a per-contest escrow PDA and can
//! only ever leave it towards entrants' vaults (payouts / refunds) or the treasury (rake,
//! capped on-chain). If the platform stops settling, anyone can refund a contest after
//! `refund_timeout_secs`.
#![allow(unexpected_cfgs)]

use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod math;
pub mod state;
pub mod token_util;

use instructions::*;

declare_id!("3TojGaWF1N3V1JFtaRWnr2Y2e8qRtNLZPX2ZV7eZbW9y");

#[cfg(not(feature = "no-entrypoint"))]
solana_security_txt::security_txt! {
    name: "bandforband.fun",
    project_url: "https://bandforband.fun",
    contacts: "email:security@bandforband.fun",
    policy: "https://bandforband.fun/docs/security",
    source_code: "https://github.com/bandforbandfun/bandforband-program"
}

#[program]
pub mod wagers {
    use super::*;

    // ---- admin ----

    pub fn initialize(ctx: Context<Initialize>, params: ConfigParams, settler: Pubkey) -> Result<()> {
        admin::initialize(ctx, params, settler)
    }

    pub fn update_config(ctx: Context<AdminOnly>, params: ConfigParams) -> Result<()> {
        admin::update_config(ctx, params)
    }

    pub fn set_settler(ctx: Context<AdminOnly>, settler: Pubkey) -> Result<()> {
        admin::set_settler(ctx, settler)
    }

    pub fn set_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
        admin::set_paused(ctx, paused)
    }

    pub fn propose_admin(ctx: Context<AdminOnly>, new_admin: Pubkey) -> Result<()> {
        admin::propose_admin(ctx, new_admin)
    }

    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        admin::accept_admin(ctx)
    }

    pub fn set_treasury(ctx: Context<SetTreasury>) -> Result<()> {
        admin::set_treasury(ctx)
    }

    // ---- vaults ----

    pub fn open_vault(ctx: Context<OpenVault>) -> Result<()> {
        vault::open_vault(ctx)
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        vault::deposit(ctx, amount)
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount: u64, fee: u64) -> Result<()> {
        vault::withdraw(ctx, amount, fee)
    }

    pub fn close_vault(ctx: Context<CloseVault>) -> Result<()> {
        vault::close_vault(ctx)
    }

    // ---- contests ----

    pub fn create_contest(
        ctx: Context<CreateContest>,
        id: [u8; 16],
        params: CreateContestParams,
    ) -> Result<()> {
        contest::create_contest(ctx, id, params)
    }

    pub fn join(ctx: Context<Join>, side: u16) -> Result<()> {
        contest::join(ctx, side)
    }

    pub fn leave(ctx: Context<Leave>) -> Result<()> {
        contest::leave(ctx)
    }

    pub fn remove_entry(ctx: Context<RemoveEntry>) -> Result<()> {
        contest::remove_entry(ctx)
    }

    pub fn lock_contest(ctx: Context<SettlerContest>) -> Result<()> {
        contest::lock_contest(ctx)
    }

    pub fn cancel(ctx: Context<Cancel>) -> Result<()> {
        contest::cancel(ctx)
    }

    pub fn settle(
        ctx: Context<Settle>,
        outcome: Outcome,
        fee_bps: u16,
        result_hash: [u8; 32],
    ) -> Result<()> {
        contest::settle(ctx, outcome, fee_bps, result_hash)
    }

    pub fn settle_book(
        ctx: Context<SettleBook>,
        entries: Vec<u16>,
        bps: Vec<u16>,
        fee_bps: u16,
        result_hash: [u8; 32],
    ) -> Result<()> {
        contest::settle_book(ctx, entries, bps, fee_bps, result_hash)
    }

    pub fn claim_entry(ctx: Context<ClaimEntry>) -> Result<()> {
        contest::claim_entry(ctx)
    }

    pub fn close_contest(ctx: Context<CloseContest>) -> Result<()> {
        contest::close_contest(ctx)
    }

    // ---- backing ----

    pub fn back(ctx: Context<Back>, side: u16, amount: u64) -> Result<()> {
        backing::back(ctx, side, amount)
    }

    pub fn claim_backing(ctx: Context<ClaimBacking>) -> Result<()> {
        backing::claim_backing(ctx)
    }

    // ---- commerce ----

    pub fn purchase(ctx: Context<Purchase>, order_id: [u8; 16], amount: u64) -> Result<()> {
        commerce::purchase(ctx, order_id, amount)
    }

    pub fn tip(ctx: Context<Tip>, order_id: [u8; 16], amount: u64, fee: u64) -> Result<()> {
        commerce::tip(ctx, order_id, amount, fee)
    }

    pub fn promo_credit(ctx: Context<PromoCredit>, order_id: [u8; 16], amount: u64) -> Result<()> {
        commerce::promo_credit(ctx, order_id, amount)
    }

    pub fn close_order(ctx: Context<CloseOrder>) -> Result<()> {
        commerce::close_order(ctx)
    }

    // ---- upgrades ----

    pub fn migrate_account(ctx: Context<MigrateAccount>) -> Result<()> {
        migrate::migrate_account(ctx)
    }
}
