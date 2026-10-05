use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::constants::*;
use crate::errors::WagersError;
use crate::events::{BackingClaimed, BackingPlaced};
use crate::instructions::contest::contest_seeds;
use crate::math::{self, BackingSettlement};
use crate::state::*;
use crate::token_util;

#[derive(Accounts)]
pub struct Back<'info> {
    #[account(mut)]
    pub rent_payer: Signer<'info>,
    pub backer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(mut, seeds = [CONTEST_SEED, contest.id.as_ref()], bump = contest.bump)]
    pub contest: Box<Account<'info, Contest>>,
    #[account(mut, seeds = [CONTEST_TOKEN_SEED, contest.id.as_ref()], bump = contest.token_bump)]
    pub contest_token: Box<Account<'info, TokenAccount>>,
    #[account(seeds = [VAULT_SEED, backer.key().as_ref()], bump = backer_vault.bump)]
    pub backer_vault: Box<Account<'info, UserVault>>,
    #[account(mut, seeds = [VAULT_TOKEN_SEED, backer.key().as_ref()], bump = backer_vault.token_bump)]
    pub backer_vault_token: Box<Account<'info, TokenAccount>>,
    #[account(
        init_if_needed,
        payer = rent_payer,
        space = 8 + Backing::INIT_SPACE,
        seeds = [BACKING_SEED, contest.key().as_ref(), backer.key().as_ref()],
        bump,
    )]
    pub backing: Box<Account<'info, Backing>>,
    /// CHECK: must not exist: players can't back (or fade) a contest they are in.
    #[account(seeds = [ENTRY_SEED, contest.key().as_ref(), backer.key().as_ref()], bump)]
    pub backer_entry: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn back(ctx: Context<Back>, side: u16, amount: u64) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, WagersError::Paused);
    require!(config.backing_enabled, WagersError::BackingDisabled);
    require!(ctx.accounts.backer_entry.data_is_empty(), WagersError::PlayerCannotBack);
    let now = Clock::get()?.unix_timestamp;
    let contest = &mut ctx.accounts.contest;
    require!(contest.backing_allowed, WagersError::BackingDisabled);
    require!(contest.status == ContestStatus::Locked, WagersError::ContestNotLocked);
    // Only during PREPARING: once the contest is LIVE nobody can bet with hindsight.
    require!(now < contest.starts_at, WagersError::BackingClosed);
    require!(side < contest.sides, WagersError::InvalidSide);
    require!(contest.side_counts[side as usize] > 0, WagersError::InvalidSide);
    require!(amount >= config.backing_min, WagersError::InvalidAmount);

    let backing = &mut ctx.accounts.backing;
    let is_new = backing.amount == 0;
    if is_new {
        backing.contest = contest.key();
        backing.backer = ctx.accounts.backer.key();
        backing.rent_payer = ctx.accounts.rent_payer.key();
        backing.side = side;
        backing.bump = ctx.bumps.backing;
        backing.version = ACCOUNT_VERSION;
    } else {
        require!(backing.side == side, WagersError::BackingSideMismatch);
    }
    let total = backing.amount.checked_add(amount).ok_or(WagersError::MathOverflow)?;
    require!(total <= config.backing_max_per_user, WagersError::BackingLimit);

    token_util::transfer(
        ctx.accounts.backer_vault_token.to_account_info(),
        ctx.accounts.contest_token.to_account_info(),
        ctx.accounts.backer_vault.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        ctx.accounts.usdc_mint.decimals,
        amount,
        Some(&[&[VAULT_SEED, ctx.accounts.backer.key().as_ref(), &[ctx.accounts.backer_vault.bump]]]),
    )?;

    backing.amount = total;
    let pool = &mut contest.backing_pools[side as usize];
    *pool = pool.checked_add(amount).ok_or(WagersError::MathOverflow)?;
    contest.backing_total = contest.backing_total.checked_add(amount).ok_or(WagersError::MathOverflow)?;
    if is_new {
        contest.open_backings += 1;
    }
    emit!(BackingPlaced { contest: contest.key(), backer: backing.backer, side, amount, total });
    Ok(())
}

#[derive(Accounts)]
pub struct ClaimBacking<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(mut, seeds = [CONTEST_SEED, contest.id.as_ref()], bump = contest.bump)]
    pub contest: Box<Account<'info, Contest>>,
    #[account(mut, seeds = [CONTEST_TOKEN_SEED, contest.id.as_ref()], bump = contest.token_bump)]
    pub contest_token: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        close = rent_receiver,
        constraint = backing.contest == contest.key() @ WagersError::WrongContest,
    )]
    pub backing: Box<Account<'info, Backing>>,
    #[account(seeds = [VAULT_SEED, backing.backer.as_ref()], bump = backer_vault.bump)]
    pub backer_vault: Box<Account<'info, UserVault>>,
    #[account(mut, seeds = [VAULT_TOKEN_SEED, backing.backer.as_ref()], bump = backer_vault.token_bump)]
    pub backer_vault_token: Box<Account<'info, TokenAccount>>,
    /// CHECK: rent goes back to whoever paid it.
    #[account(mut, address = backing.rent_payer)]
    pub rent_receiver: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

pub fn backing_payout(contest: &Contest, backing: &Backing) -> Result<(u64, bool)> {
    if contest.status.refunds() || contest.backing_refund_all {
        require!(contest.status.is_terminal(), WagersError::NotFinished);
        return Ok((backing.amount, true));
    }
    require!(contest.status == ContestStatus::Settled, WagersError::NotFinished);
    let settlement = BackingSettlement {
        refund_all: false,
        fee: 0,
        winning_pool: contest.backing_winning_pool,
        net_losing: contest.backing_net_losing,
    };
    let amount = math::backing_payout(&settlement, backing.amount, contest.is_winning_side(backing.side))
        .ok_or(WagersError::MathOverflow)?;
    Ok((amount, false))
}

/// Permissionless, like `claim_entry`.
pub fn claim_backing(ctx: Context<ClaimBacking>) -> Result<()> {
    let contest = &mut ctx.accounts.contest;
    let backing = &ctx.accounts.backing;
    let (amount, refund) = backing_payout(contest, backing)?;
    let (id, bump) = (contest.id, [contest.bump]);
    token_util::transfer(
        ctx.accounts.contest_token.to_account_info(),
        ctx.accounts.backer_vault_token.to_account_info(),
        contest.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        ctx.accounts.usdc_mint.decimals,
        amount,
        Some(&[&contest_seeds(&id, &bump)]),
    )?;
    contest.open_backings -= 1;
    emit!(BackingClaimed { contest: contest.key(), backer: backing.backer, amount, refund });
    Ok(())
}
