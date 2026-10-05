use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::constants::*;
use crate::errors::WagersError;
use crate::events::{PromoCredited, Purchased, Tipped};
use crate::math;
use crate::state::*;
use crate::token_util;

fn record_order(
    order: &mut Order,
    id: [u8; 16],
    kind: OrderKind,
    from: Pubkey,
    to: Pubkey,
    amount: u64,
    rent_payer: Pubkey,
    bump: u8,
) -> Result<()> {
    order.id = id;
    order.kind = kind;
    order.from = from;
    order.to = to;
    order.amount = amount;
    order.created_at = Clock::get()?.unix_timestamp;
    order.rent_payer = rent_payer;
    order.bump = bump;
    order.version = ACCOUNT_VERSION;
    Ok(())
}

/// Shop purchase: the order id (the DB order UUID) makes the debit idempotent, so a
/// retried request can never charge twice.
#[derive(Accounts)]
#[instruction(order_id: [u8; 16])]
pub struct Purchase<'info> {
    #[account(mut)]
    pub rent_payer: Signer<'info>,
    pub buyer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(seeds = [VAULT_SEED, buyer.key().as_ref()], bump = buyer_vault.bump)]
    pub buyer_vault: Box<Account<'info, UserVault>>,
    #[account(mut, seeds = [VAULT_TOKEN_SEED, buyer.key().as_ref()], bump = buyer_vault.token_bump)]
    pub buyer_vault_token: Box<Account<'info, TokenAccount>>,
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

pub fn purchase(ctx: Context<Purchase>, order_id: [u8; 16], amount: u64) -> Result<()> {
    require!(!ctx.accounts.config.paused, WagersError::Paused);
    require!(amount > 0, WagersError::InvalidAmount);
    let buyer = ctx.accounts.buyer.key();
    token_util::transfer(
        ctx.accounts.buyer_vault_token.to_account_info(),
        ctx.accounts.treasury_token.to_account_info(),
        ctx.accounts.buyer_vault.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        ctx.accounts.usdc_mint.decimals,
        amount,
        Some(&[&[VAULT_SEED, buyer.as_ref(), &[ctx.accounts.buyer_vault.bump]]]),
    )?;
    record_order(
        &mut ctx.accounts.order,
        order_id,
        OrderKind::Purchase,
        buyer,
        ctx.accounts.treasury_token.key(),
        amount,
        ctx.accounts.rent_payer.key(),
        ctx.bumps.order,
    )?;
    emit!(Purchased { order_id, buyer, amount });
    Ok(())
}

#[derive(Accounts)]
#[instruction(order_id: [u8; 16])]
pub struct Tip<'info> {
    #[account(mut)]
    pub rent_payer: Signer<'info>,
    pub sender: Signer<'info>,
    /// CHECK: only used to derive the recipient's vault.
    pub recipient: UncheckedAccount<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(seeds = [VAULT_SEED, sender.key().as_ref()], bump = sender_vault.bump)]
    pub sender_vault: Box<Account<'info, UserVault>>,
    #[account(mut, seeds = [VAULT_TOKEN_SEED, sender.key().as_ref()], bump = sender_vault.token_bump)]
    pub sender_vault_token: Box<Account<'info, TokenAccount>>,
    #[account(seeds = [VAULT_SEED, recipient.key().as_ref()], bump = recipient_vault.bump)]
    pub recipient_vault: Box<Account<'info, UserVault>>,
    #[account(
        mut,
        seeds = [VAULT_TOKEN_SEED, recipient.key().as_ref()],
        bump = recipient_vault.token_bump,
    )]
    pub recipient_vault_token: Box<Account<'info, TokenAccount>>,
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

pub fn tip(ctx: Context<Tip>, order_id: [u8; 16], amount: u64, fee: u64) -> Result<()> {
    require!(!ctx.accounts.config.paused, WagersError::Paused);
    require!(amount > 0, WagersError::InvalidAmount);
    let sender = ctx.accounts.sender.key();
    let recipient = ctx.accounts.recipient.key();
    require_keys_neq!(sender, recipient, WagersError::SelfTransfer);
    let max_fee = math::mul_bps(amount, MAX_FEE_BPS).ok_or(WagersError::MathOverflow)?;
    require!(fee <= max_fee, WagersError::FeeTooHigh);
    let decimals = ctx.accounts.usdc_mint.decimals;
    let seeds: &[&[u8]] = &[VAULT_SEED, sender.as_ref(), &[ctx.accounts.sender_vault.bump]];
    token_util::transfer(
        ctx.accounts.sender_vault_token.to_account_info(),
        ctx.accounts.recipient_vault_token.to_account_info(),
        ctx.accounts.sender_vault.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        decimals,
        amount,
        Some(&[seeds]),
    )?;
    token_util::transfer(
        ctx.accounts.sender_vault_token.to_account_info(),
        ctx.accounts.treasury_token.to_account_info(),
        ctx.accounts.sender_vault.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        decimals,
        fee,
        Some(&[seeds]),
    )?;
    record_order(
        &mut ctx.accounts.order,
        order_id,
        OrderKind::Tip,
        sender,
        recipient,
        amount,
        ctx.accounts.rent_payer.key(),
        ctx.bumps.order,
    )?;
    emit!(Tipped { order_id, from: sender, to: recipient, amount, fee });
    Ok(())
}

/// Promo codes, referral rewards and admin compensation are paid from the pre-funded
/// rewards pool, never from other users' funds, and capped per transaction.
#[derive(Accounts)]
#[instruction(order_id: [u8; 16])]
pub struct PromoCredit<'info> {
    #[account(mut)]
    pub rent_payer: Signer<'info>,
    pub settler: Signer<'info>,
    /// CHECK: only used to derive the recipient's vault.
    pub recipient: UncheckedAccount<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = settler @ WagersError::NotSettler)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(mut, seeds = [REWARDS_SEED], bump = config.rewards_bump)]
    pub rewards_token: Box<Account<'info, TokenAccount>>,
    #[account(seeds = [VAULT_SEED, recipient.key().as_ref()], bump = recipient_vault.bump)]
    pub recipient_vault: Box<Account<'info, UserVault>>,
    #[account(
        mut,
        seeds = [VAULT_TOKEN_SEED, recipient.key().as_ref()],
        bump = recipient_vault.token_bump,
    )]
    pub recipient_vault_token: Box<Account<'info, TokenAccount>>,
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

pub fn promo_credit(ctx: Context<PromoCredit>, order_id: [u8; 16], amount: u64) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, WagersError::Paused);
    require!(amount > 0 && amount <= config.max_promo_per_tx, WagersError::InvalidAmount);
    let recipient = ctx.accounts.recipient.key();
    token_util::transfer(
        ctx.accounts.rewards_token.to_account_info(),
        ctx.accounts.recipient_vault_token.to_account_info(),
        config.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        ctx.accounts.usdc_mint.decimals,
        amount,
        Some(&[&[CONFIG_SEED, &[config.bump]]]),
    )?;
    record_order(
        &mut ctx.accounts.order,
        order_id,
        OrderKind::Promo,
        ctx.accounts.rewards_token.key(),
        recipient,
        amount,
        ctx.accounts.rent_payer.key(),
        ctx.bumps.order,
    )?;
    emit!(PromoCredited { order_id, recipient, amount });
    Ok(())
}

#[derive(Accounts)]
pub struct CloseOrder<'info> {
    #[account(mut, close = rent_receiver)]
    pub order: Box<Account<'info, Order>>,
    /// CHECK: rent goes back to whoever paid it.
    #[account(mut, address = order.rent_payer)]
    pub rent_receiver: UncheckedAccount<'info>,
}

/// After the retention window, a replay of the same order id is impossible in practice
/// (the backend rejects ids older than a few minutes), so the rent can be reclaimed.
pub fn close_order(ctx: Context<CloseOrder>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(
        now >= ctx.accounts.order.created_at + ORDER_RETENTION_SECS,
        WagersError::OrderTooRecent
    );
    Ok(())
}
