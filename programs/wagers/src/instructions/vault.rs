use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::constants::*;
use crate::errors::WagersError;
use crate::events::{Deposited, VaultClosed, VaultOpened, Withdrawn};
use crate::state::{Config, UserVault};
use crate::token_util;

#[derive(Accounts)]
pub struct OpenVault<'info> {
    #[account(mut)]
    pub rent_payer: Signer<'info>,
    /// CHECK: a vault can be opened for any wallet (e.g. at signup, paid by the platform).
    /// Only that wallet can deposit or withdraw. The one other debit is a VIP mandate that
    /// same wallet creates: the biller can then charge its pinned amount to the treasury.
    pub owner: UncheckedAccount<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(
        init,
        payer = rent_payer,
        space = 8 + UserVault::INIT_SPACE,
        seeds = [VAULT_SEED, owner.key().as_ref()],
        bump,
    )]
    pub vault: Account<'info, UserVault>,
    #[account(
        init,
        payer = rent_payer,
        seeds = [VAULT_TOKEN_SEED, owner.key().as_ref()],
        bump,
        token::mint = usdc_mint,
        token::authority = vault,
    )]
    pub vault_token: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn open_vault(ctx: Context<OpenVault>) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    vault.owner = ctx.accounts.owner.key();
    vault.total_deposited = 0;
    vault.total_withdrawn = 0;
    vault.created_at = Clock::get()?.unix_timestamp;
    vault.bump = ctx.bumps.vault;
    vault.token_bump = ctx.bumps.vault_token;
    vault.version = ACCOUNT_VERSION;
    vault.rent_payer = ctx.accounts.rent_payer.key();
    emit!(VaultOpened { owner: vault.owner });
    Ok(())
}

#[derive(Accounts)]
pub struct CloseVault<'info> {
    pub settler: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = settler @ WagersError::NotSettler)]
    pub config: Box<Account<'info, Config>>,
    /// CHECK: only used to derive the vault.
    pub owner: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED, owner.key().as_ref()],
        bump = vault.bump,
        has_one = owner,
        close = rent_receiver,
    )]
    pub vault: Account<'info, UserVault>,
    #[account(mut, seeds = [VAULT_TOKEN_SEED, owner.key().as_ref()], bump = vault.token_bump)]
    pub vault_token: Account<'info, TokenAccount>,
    /// CHECK: receives lamports only; must be whoever paid the rent.
    #[account(mut, address = vault.rent_payer)]
    pub rent_receiver: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

/// Closes an empty vault and returns its rent to whoever paid it. The vault can be opened
/// again at any time, and anything owed to the owner reopens it before paying, so this only
/// ever reclaims rent: it can't touch funds.
pub fn close_vault(ctx: Context<CloseVault>) -> Result<()> {
    require!(ctx.accounts.vault.rent_payer != Pubkey::default(), WagersError::VaultNotClosable);
    require!(ctx.accounts.vault_token.amount == 0, WagersError::VaultNotEmpty);
    let owner_key = ctx.accounts.owner.key();
    let seeds: &[&[u8]] = &[VAULT_SEED, owner_key.as_ref(), &[ctx.accounts.vault.bump]];
    token_util::close_token_account(
        ctx.accounts.vault_token.to_account_info(),
        ctx.accounts.rent_receiver.to_account_info(),
        ctx.accounts.vault.to_account_info(),
        &[seeds],
    )?;
    emit!(VaultClosed { owner: owner_key });
    Ok(())
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(mut, seeds = [VAULT_SEED, owner.key().as_ref()], bump = vault.bump, has_one = owner)]
    pub vault: Account<'info, UserVault>,
    #[account(mut, seeds = [VAULT_TOKEN_SEED, owner.key().as_ref()], bump = vault.token_bump)]
    pub vault_token: Account<'info, TokenAccount>,
    #[account(mut, token::mint = usdc_mint, token::authority = owner)]
    pub source: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
    require!(amount > 0, WagersError::InvalidAmount);
    token_util::transfer(
        ctx.accounts.source.to_account_info(),
        ctx.accounts.vault_token.to_account_info(),
        ctx.accounts.owner.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        ctx.accounts.usdc_mint.decimals,
        amount,
        None,
    )?;
    let vault = &mut ctx.accounts.vault;
    vault.total_deposited = vault.total_deposited.checked_add(amount).ok_or(WagersError::MathOverflow)?;
    emit!(Deposited { owner: vault.owner, amount });
    Ok(())
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(mut, seeds = [VAULT_SEED, owner.key().as_ref()], bump = vault.bump, has_one = owner)]
    pub vault: Account<'info, UserVault>,
    #[account(mut, seeds = [VAULT_TOKEN_SEED, owner.key().as_ref()], bump = vault.token_bump)]
    pub vault_token: Account<'info, TokenAccount>,
    /// Any USDC account: the owner's wallet, or a Relay deposit account for bridging.
    #[account(
        mut,
        token::mint = usdc_mint,
        constraint = destination.key() != vault_token.key() @ WagersError::SelfTransfer,
    )]
    pub destination: Account<'info, TokenAccount>,
    #[account(mut, address = config.treasury_token)]
    pub treasury_token: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

/// Withdrawals are never blocked by `paused`: users can always take their money out.
pub fn withdraw(ctx: Context<Withdraw>, amount: u64, fee: u64) -> Result<()> {
    require!(amount > 0, WagersError::InvalidAmount);
    require!(fee <= ctx.accounts.config.withdraw_fee_cap, WagersError::FeeTooHigh);
    let owner_key = ctx.accounts.owner.key();
    let seeds: &[&[u8]] = &[VAULT_SEED, owner_key.as_ref(), &[ctx.accounts.vault.bump]];
    let signer = &[seeds];
    let decimals = ctx.accounts.usdc_mint.decimals;
    token_util::transfer(
        ctx.accounts.vault_token.to_account_info(),
        ctx.accounts.destination.to_account_info(),
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        decimals,
        amount,
        Some(signer),
    )?;
    token_util::transfer(
        ctx.accounts.vault_token.to_account_info(),
        ctx.accounts.treasury_token.to_account_info(),
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        decimals,
        fee,
        Some(signer),
    )?;
    let vault = &mut ctx.accounts.vault;
    vault.total_withdrawn = vault
        .total_withdrawn
        .checked_add(amount)
        .and_then(|v| v.checked_add(fee))
        .ok_or(WagersError::MathOverflow)?;
    emit!(Withdrawn { owner: owner_key, destination: ctx.accounts.destination.key(), amount, fee });
    Ok(())
}
