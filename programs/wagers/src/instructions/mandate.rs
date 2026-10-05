use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::constants::*;
use crate::errors::WagersError;
use crate::events::{MandateCancelled, MandateCreated, Purchased};
use crate::state::*;
use crate::token_util;

use super::commerce::record_order;

/// The owner arms a recurring debit. Created in the same transaction as the VIP `purchase`
/// it covers. The amount, period and first charge time are arguments the owner signs;
/// `charge_mandate` cannot change them except to roll `next_charge_at` forward.
#[derive(Accounts)]
#[instruction(kind: u8)]
pub struct CreateMandate<'info> {
    #[account(mut)]
    pub rent_payer: Signer<'info>,
    pub owner: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        init,
        payer = rent_payer,
        space = 8 + Mandate::INIT_SPACE,
        seeds = [MANDATE_SEED, owner.key().as_ref(), &[kind]],
        bump,
    )]
    pub mandate: Box<Account<'info, Mandate>>,
    pub system_program: Program<'info, System>,
}

pub fn create_mandate(
    ctx: Context<CreateMandate>,
    kind: u8,
    amount_per_period: u64,
    period_secs: i64,
    next_charge_at: i64,
) -> Result<()> {
    require!(!ctx.accounts.config.paused, WagersError::Paused);
    require!(kind == MANDATE_KIND_VIP, WagersError::InvalidMandate);
    require!(amount_per_period > 0, WagersError::InvalidAmount);
    require!(
        (MIN_MANDATE_PERIOD_SECS..=MAX_MANDATE_PERIOD_SECS).contains(&period_secs),
        WagersError::InvalidMandate
    );
    let now = Clock::get()?.unix_timestamp;
    let latest = now.checked_add(MAX_MANDATE_HORIZON_SECS).ok_or(WagersError::MathOverflow)?;
    require!(next_charge_at >= now && next_charge_at <= latest, WagersError::InvalidMandate);

    let mandate = &mut ctx.accounts.mandate;
    mandate.owner = ctx.accounts.owner.key();
    mandate.kind = kind;
    mandate.amount_per_period = amount_per_period;
    mandate.period_secs = period_secs;
    mandate.next_charge_at = next_charge_at;
    mandate.rent_payer = ctx.accounts.rent_payer.key();
    mandate.bump = ctx.bumps.mandate;
    mandate.version = ACCOUNT_VERSION;
    emit!(MandateCreated {
        owner: mandate.owner,
        kind,
        amount_per_period,
        period_secs,
        next_charge_at,
    });
    Ok(())
}

/// Biller-only. Moves `mandate.amount_per_period` to `config.treasury_token`. The biller
/// does not supply the amount or the destination: both are fixed by the account the owner
/// created and by the config treasury constraint.
#[derive(Accounts)]
#[instruction(order_id: [u8; 16])]
pub struct ChargeMandate<'info> {
    #[account(mut)]
    pub rent_payer: Signer<'info>,
    pub biller: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = biller @ WagersError::NotBiller)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(
        mut,
        seeds = [MANDATE_SEED, mandate.owner.as_ref(), &[mandate.kind]],
        bump = mandate.bump,
    )]
    pub mandate: Box<Account<'info, Mandate>>,
    /// CHECK: the vault owner. Must be the mandate owner.
    #[account(address = mandate.owner)]
    pub owner: UncheckedAccount<'info>,
    #[account(seeds = [VAULT_SEED, mandate.owner.as_ref()], bump = owner_vault.bump, has_one = owner)]
    pub owner_vault: Box<Account<'info, UserVault>>,
    #[account(mut, seeds = [VAULT_TOKEN_SEED, mandate.owner.as_ref()], bump = owner_vault.token_bump)]
    pub owner_vault_token: Box<Account<'info, TokenAccount>>,
    /// The treasury configured on-chain. A biller-supplied destination would not match this.
    #[account(mut, address = config.treasury_token)]
    pub treasury_token: Box<Account<'info, TokenAccount>>,
    #[account(
        init,
        payer = rent_payer,
        space = 8 + Order::INIT_SPACE,
        seeds = [ORDER_SEED, order_id.as_ref()],
        bump,
    )]
    pub order: Box<Account<'info, Order>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn charge_mandate(ctx: Context<ChargeMandate>, order_id: [u8; 16]) -> Result<()> {
    require!(!ctx.accounts.config.paused, WagersError::Paused);
    let now = Clock::get()?.unix_timestamp;
    let mandate = &mut ctx.accounts.mandate;
    require!(now >= mandate.next_charge_at, WagersError::MandateNotDue);
    let amount = mandate.amount_per_period;
    require!(amount > 0, WagersError::InvalidAmount);
    mandate.next_charge_at = now.checked_add(mandate.period_secs).ok_or(WagersError::MathOverflow)?;

    let owner = mandate.owner;
    token_util::transfer(
        ctx.accounts.owner_vault_token.to_account_info(),
        ctx.accounts.treasury_token.to_account_info(),
        ctx.accounts.owner_vault.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        ctx.accounts.usdc_mint.decimals,
        amount,
        Some(&[&[VAULT_SEED, owner.as_ref(), &[ctx.accounts.owner_vault.bump]]]),
    )?;
    record_order(
        &mut ctx.accounts.order,
        order_id,
        OrderKind::Purchase,
        owner,
        ctx.accounts.treasury_token.key(),
        amount,
        ctx.accounts.rent_payer.key(),
        ctx.bumps.order,
    )?;
    emit!(Purchased { order_id, buyer: owner, amount });
    Ok(())
}

/// Owner or biller. Closes the mandate and returns rent to whoever paid it. Works while
/// the program is paused, same as `withdraw`.
#[derive(Accounts)]
pub struct CancelMandate<'info> {
    pub authority: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        mut,
        seeds = [MANDATE_SEED, mandate.owner.as_ref(), &[mandate.kind]],
        bump = mandate.bump,
        close = rent_receiver,
        constraint = authority.key() == mandate.owner || authority.key() == config.biller @ WagersError::NotMandateAuthority,
    )]
    pub mandate: Box<Account<'info, Mandate>>,
    /// CHECK: rent goes back to whoever paid it.
    #[account(mut, address = mandate.rent_payer)]
    pub rent_receiver: UncheckedAccount<'info>,
}

pub fn cancel_mandate(ctx: Context<CancelMandate>) -> Result<()> {
    let mandate = &ctx.accounts.mandate;
    emit!(MandateCancelled { owner: mandate.owner, kind: mandate.kind, by: ctx.accounts.authority.key() });
    Ok(())
}
