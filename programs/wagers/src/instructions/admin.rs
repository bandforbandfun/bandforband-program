use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::constants::*;
use crate::errors::WagersError;
use crate::events::ConfigUpdated;
use crate::math::{self, Tier};
use crate::state::{Config, FeeTier};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct ConfigParams {
    pub fee_tiers: Vec<FeeTier>,
    pub backing_fee_bps: u16,
    pub backing_enabled: bool,
    pub backing_min: u64,
    pub backing_max_per_user: u64,
    pub preparation_secs: i64,
    pub refund_timeout_secs: i64,
    pub withdraw_fee_cap: u64,
    pub max_promo_per_tx: u64,
}

impl ConfigParams {
    pub fn validate(&self) -> Result<()> {
        let tiers: Vec<Tier> = self.fee_tiers.iter().map(|t| (*t).into()).collect();
        require!(tiers.len() <= MAX_FEE_TIERS, WagersError::InvalidConfig);
        require!(math::validate_tiers(&tiers, MAX_FEE_BPS), WagersError::InvalidConfig);
        require!(self.backing_fee_bps <= MAX_FEE_BPS, WagersError::FeeTooHigh);
        require!(self.backing_min > 0, WagersError::InvalidConfig);
        require!(self.backing_max_per_user >= self.backing_min, WagersError::InvalidConfig);
        require!(
            (0..=MAX_PREPARATION_SECS).contains(&self.preparation_secs),
            WagersError::InvalidConfig
        );
        require!(self.refund_timeout_secs >= MIN_REFUND_TIMEOUT_SECS, WagersError::InvalidConfig);
        Ok(())
    }

    pub fn apply(&self, config: &mut Config) {
        let mut tiers = [FeeTier::default(); MAX_FEE_TIERS];
        for (i, tier) in self.fee_tiers.iter().enumerate() {
            tiers[i] = *tier;
        }
        config.fee_tiers = tiers;
        config.fee_tier_count = self.fee_tiers.len() as u8;
        config.backing_fee_bps = self.backing_fee_bps;
        config.backing_enabled = self.backing_enabled;
        config.backing_min = self.backing_min;
        config.backing_max_per_user = self.backing_max_per_user;
        config.preparation_secs = self.preparation_secs;
        config.refund_timeout_secs = self.refund_timeout_secs;
        config.withdraw_fee_cap = self.withdraw_fee_cap;
        config.max_promo_per_tx = self.max_promo_per_tx;
    }
}

fn emit_config(config: &Config) {
    emit!(ConfigUpdated {
        admin: config.admin,
        settler: config.settler,
        paused: config.paused,
        backing_enabled: config.backing_enabled,
    });
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(init, payer = admin, space = 8 + Config::INIT_SPACE, seeds = [CONFIG_SEED], bump)]
    pub config: Box<Account<'info, Config>>,
    pub usdc_mint: Account<'info, Mint>,
    #[account(token::mint = usdc_mint)]
    pub treasury_token: Account<'info, TokenAccount>,
    #[account(
        init,
        payer = admin,
        seeds = [REWARDS_SEED],
        bump,
        token::mint = usdc_mint,
        token::authority = config,
    )]
    pub rewards_token: Account<'info, TokenAccount>,
    /// Only the program's upgrade authority can initialize, so nobody can front-run the
    /// deployment and take the admin role.
    #[account(constraint = program.programdata_address()? == Some(program_data.key()) @ WagersError::NotAdmin)]
    pub program: Program<'info, crate::program::Wagers>,
    #[account(constraint = program_data.upgrade_authority_address == Some(admin.key()) @ WagersError::NotAdmin)]
    pub program_data: Account<'info, ProgramData>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn initialize(ctx: Context<Initialize>, params: ConfigParams, settler: Pubkey) -> Result<()> {
    params.validate()?;
    let config = &mut ctx.accounts.config;
    config.admin = ctx.accounts.admin.key();
    config.pending_admin = Pubkey::default();
    config.settler = settler;
    config.treasury_token = ctx.accounts.treasury_token.key();
    config.usdc_mint = ctx.accounts.usdc_mint.key();
    config.paused = false;
    config.bump = ctx.bumps.config;
    config.rewards_bump = ctx.bumps.rewards_token;
    config.version = ACCOUNT_VERSION;
    params.apply(config);
    emit_config(config);
    Ok(())
}

#[derive(Accounts)]
pub struct AdminOnly<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ WagersError::NotAdmin)]
    pub config: Box<Account<'info, Config>>,
}

pub fn update_config(ctx: Context<AdminOnly>, params: ConfigParams) -> Result<()> {
    params.validate()?;
    params.apply(&mut ctx.accounts.config);
    emit_config(&ctx.accounts.config);
    Ok(())
}

pub fn set_settler(ctx: Context<AdminOnly>, settler: Pubkey) -> Result<()> {
    require!(settler != Pubkey::default(), WagersError::InvalidConfig);
    require!(settler != ctx.accounts.config.biller, WagersError::InvalidConfig);
    ctx.accounts.config.settler = settler;
    emit_config(&ctx.accounts.config);
    Ok(())
}

/// Installs the mandate biller. Refuses the settler key so the two roles cannot collapse
/// into one signer.
pub fn set_biller(ctx: Context<AdminOnly>, biller: Pubkey) -> Result<()> {
    require!(biller != Pubkey::default(), WagersError::InvalidConfig);
    require!(biller != ctx.accounts.config.settler, WagersError::InvalidConfig);
    ctx.accounts.config.biller = biller;
    Ok(())
}

pub fn set_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
    ctx.accounts.config.paused = paused;
    emit_config(&ctx.accounts.config);
    Ok(())
}

pub fn propose_admin(ctx: Context<AdminOnly>, new_admin: Pubkey) -> Result<()> {
    ctx.accounts.config.pending_admin = new_admin;
    Ok(())
}

#[derive(Accounts)]
pub struct AcceptAdmin<'info> {
    pub new_admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
}

pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    require!(config.pending_admin != Pubkey::default(), WagersError::NoPendingAdmin);
    require_keys_eq!(config.pending_admin, ctx.accounts.new_admin.key(), WagersError::NotAdmin);
    config.admin = config.pending_admin;
    config.pending_admin = Pubkey::default();
    emit_config(config);
    Ok(())
}

#[derive(Accounts)]
pub struct SetTreasury<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ WagersError::NotAdmin)]
    pub config: Box<Account<'info, Config>>,
    #[account(token::mint = config.usdc_mint)]
    pub treasury_token: Account<'info, TokenAccount>,
}

pub fn set_treasury(ctx: Context<SetTreasury>) -> Result<()> {
    ctx.accounts.config.treasury_token = ctx.accounts.treasury_token.key();
    Ok(())
}
