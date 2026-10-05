use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::constants::*;
use crate::errors::WagersError;
use crate::events::*;
use crate::math;
use crate::state::*;
use crate::token_util;

/// After the join deadline the settler still has this long to start a lobby before anyone
/// may cancel it.
pub const LOCK_GRACE_SECS: i64 = 120;
/// Once full, a contest waits this long for every player to ready up. The settler then locks
/// it, or removes the absent seats and reopens it.
pub const READY_WINDOW_SECS: i64 = 180;

pub const CANCEL_BY_CREATOR: u8 = 0;
pub const CANCEL_BY_SETTLER: u8 = 1;
pub const CANCEL_JOIN_EXPIRED: u8 = 2;
pub const CANCEL_REFUND_TIMEOUT: u8 = 3;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct CreateContestParams {
    pub kind: ContestKind,
    /// Per-seat stake in micro-USDC.
    pub stake: u64,
    /// Total seats. Duel: 2 * team size. Lobby: players (one side each). Tournament: entrants.
    pub capacity: u16,
    pub min_entrants: u16,
    pub join_deadline: i64,
    pub duration: i64,
    pub backing: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub enum Outcome {
    /// Every seat on a side whose bit is set shares the net pot equally.
    Sides { mask: u64 },
    /// `entries[i]` (entry index) takes `bps[i]` of the net pot. Entries must be passed as
    /// remaining accounts in the same order.
    Placements { entries: Vec<u16>, bps: Vec<u16> },
    /// Everyone is refunded, no fee.
    Draw,
}

pub fn contest_seeds<'a>(id: &'a [u8; 16], bump: &'a [u8; 1]) -> [&'a [u8]; 3] {
    [CONTEST_SEED, id.as_ref(), bump.as_ref()]
}

fn validate_params(p: &CreateContestParams, now: i64) -> Result<(u16, u16)> {
    require!(p.stake > 0, WagersError::InvalidAmount);
    require!(p.duration > 0 && p.duration <= MAX_DURATION_SECS, WagersError::InvalidContestParams);
    require!(
        p.join_deadline > now && p.join_deadline - now <= MAX_JOIN_WINDOW_SECS,
        WagersError::InvalidContestParams
    );
    let (sides, seats) = match p.kind {
        ContestKind::Duel => {
            require!(
                p.capacity >= 2 && p.capacity % 2 == 0 && p.capacity <= 16,
                WagersError::InvalidContestParams
            );
            require!(p.min_entrants == p.capacity, WagersError::InvalidContestParams);
            (2, p.capacity / 2)
        }
        ContestKind::Lobby => {
            require!(
                p.capacity >= 2 && p.capacity as usize <= MAX_SIDES,
                WagersError::InvalidContestParams
            );
            (p.capacity, 1)
        }
        ContestKind::Tournament => {
            require!(p.capacity >= 2 && p.capacity <= MAX_CAPACITY, WagersError::InvalidContestParams);
            (1, p.capacity)
        }
    };
    require!(
        p.min_entrants >= 2 && p.min_entrants <= p.capacity,
        WagersError::InvalidContestParams
    );
    p.stake.checked_mul(p.capacity as u64).ok_or(WagersError::MathOverflow)?;
    Ok((sides, seats))
}

/// Past this the settler can no longer start an open contest, and anyone may cancel it.
fn lock_expiry(contest: &Contest) -> i64 {
    let ready = if contest.filled_at > 0 { contest.filled_at + READY_WINDOW_SECS } else { 0 };
    contest.join_deadline.max(ready) + LOCK_GRACE_SECS
}

/// Sends a seat's stake back to its payer's vault while the contest is still open.
fn refund_seat<'info>(
    contest: &mut Account<'info, Contest>,
    entry: &Entry,
    contest_token: AccountInfo<'info>,
    payer_vault_token: AccountInfo<'info>,
    usdc_mint: &Account<'info, Mint>,
) -> Result<()> {
    require!(contest.status == ContestStatus::Open, WagersError::ContestNotOpen);
    let (id, bump) = (contest.id, [contest.bump]);
    token_util::transfer(
        contest_token,
        payer_vault_token,
        contest.to_account_info(),
        usdc_mint.to_account_info(),
        usdc_mint.decimals,
        entry.stake,
        Some(&[&contest_seeds(&id, &bump)]),
    )?;
    contest.entrants -= 1;
    contest.open_entries -= 1;
    contest.side_counts[entry.side as usize] -= 1;
    contest.pot = contest.pot.checked_sub(entry.stake).ok_or(WagersError::MathOverflow)?;
    // The seat reopens, so the ready window starts over when it fills again.
    contest.filled_at = 0;
    emit!(Left { contest: contest.key(), player: entry.player, refunded: entry.stake });
    Ok(())
}

fn lock(contest: &mut Account<Contest>, config: &Config, now: i64) {
    let prep = if contest.backing_allowed { config.preparation_secs } else { 0 };
    contest.status = ContestStatus::Locked;
    contest.filled_at = now;
    contest.starts_at = now + prep;
    contest.ends_at = contest.starts_at + contest.duration;
    emit!(ContestLocked {
        contest: contest.key(),
        starts_at: contest.starts_at,
        ends_at: contest.ends_at,
        entrants: contest.entrants,
    });
}

// ---------------------------------------------------------------------------------------
// create_contest
// ---------------------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(id: [u8; 16])]
pub struct CreateContest<'info> {
    #[account(mut)]
    pub rent_payer: Signer<'info>,
    pub creator: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(
        init,
        payer = rent_payer,
        space = 8 + Contest::INIT_SPACE,
        seeds = [CONTEST_SEED, id.as_ref()],
        bump,
    )]
    pub contest: Box<Account<'info, Contest>>,
    #[account(
        init,
        payer = rent_payer,
        seeds = [CONTEST_TOKEN_SEED, id.as_ref()],
        bump,
        token::mint = usdc_mint,
        token::authority = contest,
    )]
    pub contest_token: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn create_contest(ctx: Context<CreateContest>, id: [u8; 16], params: CreateContestParams) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, WagersError::Paused);
    let now = Clock::get()?.unix_timestamp;
    let (sides, seats) = validate_params(&params, now)?;
    let contest = &mut ctx.accounts.contest;
    contest.id = id;
    contest.creator = ctx.accounts.creator.key();
    contest.rent_payer = ctx.accounts.rent_payer.key();
    contest.kind = params.kind;
    contest.status = ContestStatus::Open;
    contest.stake = params.stake;
    contest.capacity = params.capacity;
    contest.sides = sides;
    contest.seats_per_side = seats;
    contest.min_entrants = params.min_entrants;
    contest.join_deadline = params.join_deadline;
    contest.duration = params.duration;
    contest.backing_allowed =
        params.backing && config.backing_enabled && params.kind != ContestKind::Tournament;
    // Upper bound for the rake, fixed when the contest is created so a config change can
    // never raise the fee on stakes already escrowed.
    contest.fee_bps = math::tier_bps(&config.tiers(), params.stake);
    contest.bump = ctx.bumps.contest;
    contest.token_bump = ctx.bumps.contest_token;
    contest.version = ACCOUNT_VERSION;
    emit!(ContestCreated {
        contest: contest.key(),
        id,
        creator: contest.creator,
        stake: contest.stake,
        capacity: contest.capacity,
        sides,
        join_deadline: contest.join_deadline,
        duration: contest.duration,
        backing_allowed: contest.backing_allowed,
    });
    Ok(())
}

// ---------------------------------------------------------------------------------------
// join / leave
// ---------------------------------------------------------------------------------------

#[derive(Accounts)]
pub struct Join<'info> {
    #[account(mut)]
    pub rent_payer: Signer<'info>,
    /// Vault owner paying the stake. Receives the payout of this seat.
    pub payer: Signer<'info>,
    /// CHECK: the wallet taking the seat. Equal to `payer` unless a teammate covers it.
    pub player: UncheckedAccount<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(mut, seeds = [CONTEST_SEED, contest.id.as_ref()], bump = contest.bump)]
    pub contest: Box<Account<'info, Contest>>,
    #[account(mut, seeds = [CONTEST_TOKEN_SEED, contest.id.as_ref()], bump = contest.token_bump)]
    pub contest_token: Box<Account<'info, TokenAccount>>,
    #[account(seeds = [VAULT_SEED, payer.key().as_ref()], bump = payer_vault.bump)]
    pub payer_vault: Box<Account<'info, UserVault>>,
    #[account(mut, seeds = [VAULT_TOKEN_SEED, payer.key().as_ref()], bump = payer_vault.token_bump)]
    pub payer_vault_token: Box<Account<'info, TokenAccount>>,
    #[account(
        init,
        payer = rent_payer,
        space = 8 + Entry::INIT_SPACE,
        seeds = [ENTRY_SEED, contest.key().as_ref(), player.key().as_ref()],
        bump,
    )]
    pub entry: Box<Account<'info, Entry>>,
    /// CHECK: must not exist: a wallet can't both play and back the same contest.
    #[account(seeds = [BACKING_SEED, contest.key().as_ref(), player.key().as_ref()], bump)]
    pub player_backing: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn join(ctx: Context<Join>, side: u16) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, WagersError::Paused);
    require!(ctx.accounts.player_backing.data_is_empty(), WagersError::PlayerCannotBack);
    let now = Clock::get()?.unix_timestamp;
    let contest = &mut ctx.accounts.contest;
    require!(contest.status == ContestStatus::Open, WagersError::ContestNotOpen);
    require!(now <= contest.join_deadline, WagersError::JoinDeadlinePassed);
    require!(contest.entrants < contest.capacity, WagersError::ContestFull);
    require!(side < contest.sides, WagersError::InvalidSide);
    require!(
        contest.side_counts[side as usize] < contest.seats_per_side,
        WagersError::SideFull
    );

    token_util::transfer(
        ctx.accounts.payer_vault_token.to_account_info(),
        ctx.accounts.contest_token.to_account_info(),
        ctx.accounts.payer_vault.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        ctx.accounts.usdc_mint.decimals,
        contest.stake,
        Some(&[&[VAULT_SEED, ctx.accounts.payer.key().as_ref(), &[ctx.accounts.payer_vault.bump]]]),
    )?;

    let entry = &mut ctx.accounts.entry;
    entry.contest = contest.key();
    entry.player = ctx.accounts.player.key();
    entry.payer = ctx.accounts.payer.key();
    entry.rent_payer = ctx.accounts.rent_payer.key();
    entry.side = side;
    entry.index = contest.next_index;
    entry.stake = contest.stake;
    entry.bump = ctx.bumps.entry;
    entry.version = ACCOUNT_VERSION;

    contest.next_index += 1;
    contest.entrants += 1;
    contest.open_entries += 1;
    contest.side_counts[side as usize] += 1;
    contest.pot = contest.pot.checked_add(contest.stake).ok_or(WagersError::MathOverflow)?;

    emit!(Joined {
        contest: contest.key(),
        player: entry.player,
        payer: entry.payer,
        side,
        index: entry.index,
        stake: entry.stake,
    });

    if contest.entrants == contest.capacity {
        // Tournaments start as soon as they fill. Other contests wait for every player to
        // ready up, then the settler locks them.
        if contest.kind == ContestKind::Tournament {
            lock(contest, config, now);
        } else {
            contest.filled_at = now;
        }
    }
    Ok(())
}

#[derive(Accounts)]
pub struct Leave<'info> {
    /// The seat's payer (who gets the refund).
    pub payer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(mut, seeds = [CONTEST_SEED, contest.id.as_ref()], bump = contest.bump)]
    pub contest: Box<Account<'info, Contest>>,
    #[account(mut, seeds = [CONTEST_TOKEN_SEED, contest.id.as_ref()], bump = contest.token_bump)]
    pub contest_token: Box<Account<'info, TokenAccount>>,
    #[account(seeds = [VAULT_SEED, payer.key().as_ref()], bump = payer_vault.bump)]
    pub payer_vault: Box<Account<'info, UserVault>>,
    #[account(mut, seeds = [VAULT_TOKEN_SEED, payer.key().as_ref()], bump = payer_vault.token_bump)]
    pub payer_vault_token: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        close = rent_receiver,
        has_one = payer,
        constraint = entry.contest == contest.key() @ WagersError::WrongContest,
    )]
    pub entry: Box<Account<'info, Entry>>,
    /// CHECK: rent goes back to whoever paid it.
    #[account(mut, address = entry.rent_payer)]
    pub rent_receiver: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

pub fn leave(ctx: Context<Leave>) -> Result<()> {
    refund_seat(
        &mut ctx.accounts.contest,
        &ctx.accounts.entry,
        ctx.accounts.contest_token.to_account_info(),
        ctx.accounts.payer_vault_token.to_account_info(),
        &ctx.accounts.usdc_mint,
    )
}

#[derive(Accounts)]
pub struct RemoveEntry<'info> {
    pub settler: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = settler @ WagersError::NotSettler)]
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
        constraint = entry.contest == contest.key() @ WagersError::WrongContest,
    )]
    pub entry: Box<Account<'info, Entry>>,
    #[account(seeds = [VAULT_SEED, entry.payer.as_ref()], bump = payer_vault.bump)]
    pub payer_vault: Box<Account<'info, UserVault>>,
    #[account(mut, seeds = [VAULT_TOKEN_SEED, entry.payer.as_ref()], bump = payer_vault.token_bump)]
    pub payer_vault_token: Box<Account<'info, TokenAccount>>,
    /// CHECK: rent goes back to whoever paid it.
    #[account(mut, address = entry.rent_payer)]
    pub rent_receiver: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

/// The settler refunds a seat of an open contest, e.g. a player who didn't ready up in time.
/// The stake can only go back to the seat's payer.
pub fn remove_entry(ctx: Context<RemoveEntry>) -> Result<()> {
    refund_seat(
        &mut ctx.accounts.contest,
        &ctx.accounts.entry,
        ctx.accounts.contest_token.to_account_info(),
        ctx.accounts.payer_vault_token.to_account_info(),
        &ctx.accounts.usdc_mint,
    )
}

// ---------------------------------------------------------------------------------------
// lock / cancel
// ---------------------------------------------------------------------------------------

#[derive(Accounts)]
pub struct SettlerContest<'info> {
    pub settler: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = settler @ WagersError::NotSettler)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [CONTEST_SEED, contest.id.as_ref()], bump = contest.bump)]
    pub contest: Box<Account<'info, Contest>>,
}

/// Starts a contest once its players are ready: a full duel, or a lobby or tournament with
/// enough entrants. Duels need every seat (`min_entrants == capacity`).
pub fn lock_contest(ctx: Context<SettlerContest>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let contest = &mut ctx.accounts.contest;
    require!(contest.status == ContestStatus::Open, WagersError::ContestNotOpen);
    require!(now <= lock_expiry(contest), WagersError::JoinDeadlinePassed);
    require!(
        contest.entrants >= contest.min_entrants && contest.entrants >= 2,
        WagersError::NotEnoughEntrants
    );
    // A lobby with fewer players than sides can still settle: empty sides can't win.
    lock(contest, &ctx.accounts.config, now);
    Ok(())
}

#[derive(Accounts)]
pub struct Cancel<'info> {
    pub caller: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [CONTEST_SEED, contest.id.as_ref()], bump = contest.bump)]
    pub contest: Box<Account<'info, Contest>>,
}

pub fn cancel(ctx: Context<Cancel>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let caller = ctx.accounts.caller.key();
    let config = &ctx.accounts.config;
    let contest = &mut ctx.accounts.contest;
    let is_settler = caller == config.settler;
    let reason = match contest.status {
        ContestStatus::Open if is_settler => CANCEL_BY_SETTLER,
        ContestStatus::Open if caller == contest.creator => CANCEL_BY_CREATOR,
        ContestStatus::Open if now > lock_expiry(contest) => CANCEL_JOIN_EXPIRED,
        ContestStatus::Locked if is_settler => CANCEL_BY_SETTLER,
        // Liveness guarantee: if the platform disappears, anyone can release the escrow.
        ContestStatus::Locked if now > contest.ends_at + config.refund_timeout_secs => {
            CANCEL_REFUND_TIMEOUT
        }
        _ => return err!(WagersError::CannotCancel),
    };
    contest.status = ContestStatus::Cancelled;
    contest.backing_refund_all = true;
    emit!(ContestCancelled { contest: contest.key(), by: caller, reason });
    Ok(())
}

// ---------------------------------------------------------------------------------------
// settle
// ---------------------------------------------------------------------------------------

#[derive(Accounts)]
pub struct Settle<'info> {
    pub settler: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = settler @ WagersError::NotSettler)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(mut, seeds = [CONTEST_SEED, contest.id.as_ref()], bump = contest.bump)]
    pub contest: Box<Account<'info, Contest>>,
    #[account(mut, seeds = [CONTEST_TOKEN_SEED, contest.id.as_ref()], bump = contest.token_bump)]
    pub contest_token: Box<Account<'info, TokenAccount>>,
    #[account(mut, address = config.treasury_token)]
    pub treasury_token: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

fn read_entry(info: &AccountInfo, contest: &Pubkey) -> Result<Entry> {
    require_keys_eq!(*info.owner, crate::ID, WagersError::WrongContest);
    let data = info.try_borrow_data()?;
    let entry = Entry::try_deserialize(&mut &data[..])?;
    require_keys_eq!(entry.contest, *contest, WagersError::WrongContest);
    Ok(entry)
}

pub fn settle(ctx: Context<Settle>, outcome: Outcome, fee_bps: u16, result_hash: [u8; 32]) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let backing_fee_bps = ctx.accounts.config.backing_fee_bps;
    let contest_key = ctx.accounts.contest.key();
    let contest = &mut ctx.accounts.contest;
    require!(contest.status == ContestStatus::Locked, WagersError::ContestNotLocked);
    require!(now >= contest.starts_at, WagersError::NotStarted);
    require!(fee_bps <= contest.fee_bps, WagersError::FeeTooHigh);

    let mut to_treasury: u64 = 0;
    let mut backing_mask: u64 = 0;
    match outcome {
        Outcome::Draw => {
            contest.status = ContestStatus::Refunded;
            contest.outcome_kind = OUTCOME_DRAW;
            contest.fee_bps = 0;
            contest.backing_refund_all = true;
        }
        Outcome::Sides { mask } => {
            require!(contest.kind != ContestKind::Tournament, WagersError::InvalidOutcome);
            let seats = math::winning_seats(&contest.side_counts, contest.sides, mask)
                .ok_or(WagersError::InvalidOutcome)?;
            let s = math::settle_sides(contest.pot, fee_bps, seats).ok_or(WagersError::MathOverflow)?;
            contest.status = ContestStatus::Settled;
            contest.outcome_kind = OUTCOME_SIDES;
            contest.winning_mask = mask;
            contest.per_seat_payout = s.per_seat;
            contest.net_pot = contest.pot - s.fee - s.dust;
            contest.fee_bps = fee_bps;
            contest.fee_paid = s.fee;
            to_treasury += s.fee + s.dust;
            backing_mask = mask;
        }
        Outcome::Placements { entries, bps } => {
            require!(contest.kind != ContestKind::Duel, WagersError::InvalidOutcome);
            require!(
                entries.len() == bps.len()
                    && math::validate_placements(&bps, MAX_PLACES)
                    && entries.len() <= contest.entrants as usize,
                WagersError::InvalidOutcome
            );
            require!(ctx.remaining_accounts.len() == entries.len(), WagersError::InvalidOutcome);
            let mut placement_entries = [0u16; MAX_PLACES];
            let mut placement_bps = [0u16; MAX_PLACES];
            for (place, index) in entries.iter().enumerate() {
                require!(!entries[..place].contains(index), WagersError::InvalidOutcome);
                let entry = read_entry(&ctx.remaining_accounts[place], &contest_key)?;
                require!(entry.index == *index, WagersError::InvalidOutcome);
                if place == 0 && entry.side < 64 {
                    backing_mask = 1u64 << entry.side;
                }
                placement_entries[place] = *index;
                placement_bps[place] = bps[place];
            }
            let fee = math::mul_bps(contest.pot, fee_bps).ok_or(WagersError::MathOverflow)?;
            contest.status = ContestStatus::Settled;
            contest.outcome_kind = OUTCOME_PLACEMENTS;
            contest.placement_entries = placement_entries;
            contest.placement_bps = placement_bps;
            contest.placement_count = entries.len() as u8;
            contest.net_pot = contest.pot - fee;
            contest.fee_bps = fee_bps;
            contest.fee_paid = fee;
            to_treasury += fee;
        }
    }

    let mut backing_fee = 0;
    if contest.status == ContestStatus::Settled && contest.backing_total > 0 {
        let b = math::settle_backing(&contest.backing_pools[..contest.sides as usize], backing_mask, backing_fee_bps)
            .ok_or(WagersError::MathOverflow)?;
        contest.backing_refund_all = b.refund_all;
        contest.backing_winning_pool = b.winning_pool;
        contest.backing_net_losing = b.net_losing;
        contest.winning_mask = backing_mask;
        backing_fee = b.fee;
        to_treasury += b.fee;
    }
    contest.result_hash = result_hash;

    let (id, bump) = (contest.id, [contest.bump]);
    token_util::transfer(
        ctx.accounts.contest_token.to_account_info(),
        ctx.accounts.treasury_token.to_account_info(),
        contest.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        ctx.accounts.usdc_mint.decimals,
        to_treasury,
        Some(&[&contest_seeds(&id, &bump)]),
    )?;
    emit!(ContestSettled {
        contest: contest_key,
        outcome_kind: contest.outcome_kind,
        winning_mask: contest.winning_mask,
        fee: contest.fee_paid,
        backing_fee,
        result_hash,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct SettleBook<'info> {
    /// Must be the contest's rent payer: the book's rent goes back with the contest's.
    #[account(mut)]
    pub rent_payer: Signer<'info>,
    pub settler: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = settler @ WagersError::NotSettler)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(mut, seeds = [CONTEST_SEED, contest.id.as_ref()], bump = contest.bump)]
    pub contest: Box<Account<'info, Contest>>,
    #[account(mut, seeds = [CONTEST_TOKEN_SEED, contest.id.as_ref()], bump = contest.token_bump)]
    pub contest_token: Box<Account<'info, TokenAccount>>,
    #[account(mut, address = config.treasury_token)]
    pub treasury_token: Box<Account<'info, TokenAccount>>,
    #[account(
        init,
        payer = rent_payer,
        space = 8 + PlacementBook::INIT_SPACE,
        seeds = [PLACEMENTS_SEED, contest.key().as_ref()],
        bump,
    )]
    pub book: Box<Account<'info, PlacementBook>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// Tournament settlement paying up to `MAX_BOOK_PLACES` seats. The places are written to a
/// `PlacementBook` instead of the contest account, so no entry accounts are passed: indexes
/// are checked against the seats ever taken, and tournaments carry no backing.
pub fn settle_book(
    ctx: Context<SettleBook>,
    entries: Vec<u16>,
    bps: Vec<u16>,
    fee_bps: u16,
    result_hash: [u8; 32],
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let contest_key = ctx.accounts.contest.key();
    let contest = &mut ctx.accounts.contest;
    require!(contest.status == ContestStatus::Locked, WagersError::ContestNotLocked);
    require!(now >= contest.starts_at, WagersError::NotStarted);
    require!(fee_bps <= contest.fee_bps, WagersError::FeeTooHigh);
    require!(
        contest.kind == ContestKind::Tournament && contest.backing_total == 0,
        WagersError::InvalidOutcome
    );
    require_keys_eq!(ctx.accounts.rent_payer.key(), contest.rent_payer, WagersError::InvalidOutcome);
    require!(
        entries.len() == bps.len()
            && math::validate_placements(&bps, MAX_BOOK_PLACES)
            && entries.len() <= contest.entrants as usize,
        WagersError::InvalidOutcome
    );
    for (place, index) in entries.iter().enumerate() {
        require!(
            *index < contest.next_index && !entries[..place].contains(index),
            WagersError::InvalidOutcome
        );
    }

    let fee = math::mul_bps(contest.pot, fee_bps).ok_or(WagersError::MathOverflow)?;
    contest.status = ContestStatus::Settled;
    contest.outcome_kind = OUTCOME_BOOK;
    contest.placement_count = 0;
    contest.net_pot = contest.pot - fee;
    contest.fee_bps = fee_bps;
    contest.fee_paid = fee;
    contest.result_hash = result_hash;

    let book = &mut ctx.accounts.book;
    book.contest = contest_key;
    book.count = entries.len() as u16;
    book.entries[..entries.len()].copy_from_slice(&entries);
    book.bps[..bps.len()].copy_from_slice(&bps);
    book.bump = ctx.bumps.book;
    book.version = ACCOUNT_VERSION;

    let (id, bump) = (contest.id, [contest.bump]);
    token_util::transfer(
        ctx.accounts.contest_token.to_account_info(),
        ctx.accounts.treasury_token.to_account_info(),
        contest.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        ctx.accounts.usdc_mint.decimals,
        fee,
        Some(&[&contest_seeds(&id, &bump)]),
    )?;
    emit!(ContestSettled {
        contest: contest_key,
        outcome_kind: OUTCOME_BOOK,
        winning_mask: 0,
        fee,
        backing_fee: 0,
        result_hash,
    });
    Ok(())
}

/// The contest's placement book, passed as the first remaining account. Only `settle_book`
/// creates one (at its contest's PDA), so owner, discriminator and `contest` identify it.
fn read_book(remaining: &[AccountInfo], contest: &Pubkey) -> Result<PlacementBook> {
    let info = remaining.first().ok_or(WagersError::MissingPlacementBook)?;
    require_keys_eq!(*info.owner, crate::ID, WagersError::MissingPlacementBook);
    let data = info.try_borrow_data()?;
    let book = PlacementBook::try_deserialize(&mut &data[..])?;
    require_keys_eq!(book.contest, *contest, WagersError::MissingPlacementBook);
    Ok(book)
}

// ---------------------------------------------------------------------------------------
// claim_entry / close_contest
// ---------------------------------------------------------------------------------------

#[derive(Accounts)]
pub struct ClaimEntry<'info> {
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
        constraint = entry.contest == contest.key() @ WagersError::WrongContest,
    )]
    pub entry: Box<Account<'info, Entry>>,
    #[account(seeds = [VAULT_SEED, entry.payer.as_ref()], bump = payer_vault.bump)]
    pub payer_vault: Box<Account<'info, UserVault>>,
    #[account(mut, seeds = [VAULT_TOKEN_SEED, entry.payer.as_ref()], bump = payer_vault.token_bump)]
    pub payer_vault_token: Box<Account<'info, TokenAccount>>,
    /// CHECK: rent goes back to whoever paid it.
    #[account(mut, address = entry.rent_payer)]
    pub rent_receiver: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

pub fn entry_payout(contest: &Contest, entry: &Entry) -> Result<(u64, bool)> {
    if contest.status.refunds() {
        return Ok((entry.stake, true));
    }
    require!(contest.status == ContestStatus::Settled, WagersError::NotFinished);
    let amount = match contest.outcome_kind {
        OUTCOME_SIDES if contest.is_winning_side(entry.side) => contest.per_seat_payout,
        OUTCOME_PLACEMENTS => match contest.placement_of(entry.index) {
            Some(place) => math::placement_payout(
                contest.net_pot,
                &contest.placement_bps[..contest.placement_count as usize],
                place,
            )
            .ok_or(WagersError::MathOverflow)?,
            None => 0,
        },
        OUTCOME_BOOK => return err!(WagersError::MissingPlacementBook),
        _ => 0,
    };
    Ok((amount, false))
}

/// Permissionless: the worker cranks this for every entry, users can also do it themselves.
/// Contests settled with `settle_book` need their book as the first remaining account.
pub fn claim_entry(ctx: Context<ClaimEntry>) -> Result<()> {
    let contest_key = ctx.accounts.contest.key();
    let contest = &mut ctx.accounts.contest;
    let entry = &ctx.accounts.entry;
    let (amount, refund) = if contest.status == ContestStatus::Settled && contest.outcome_kind == OUTCOME_BOOK {
        let book = read_book(ctx.remaining_accounts, &contest_key)?;
        (book.payout(contest.net_pot, entry.index).ok_or(WagersError::MathOverflow)?, false)
    } else {
        entry_payout(contest, entry)?
    };
    let (id, bump) = (contest.id, [contest.bump]);
    token_util::transfer(
        ctx.accounts.contest_token.to_account_info(),
        ctx.accounts.payer_vault_token.to_account_info(),
        contest.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        ctx.accounts.usdc_mint.decimals,
        amount,
        Some(&[&contest_seeds(&id, &bump)]),
    )?;
    contest.open_entries -= 1;
    emit!(EntryClaimed {
        contest: contest.key(),
        player: entry.player,
        payer: entry.payer,
        amount,
        refund,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct CloseContest<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(
        mut,
        close = rent_receiver,
        seeds = [CONTEST_SEED, contest.id.as_ref()],
        bump = contest.bump,
    )]
    pub contest: Box<Account<'info, Contest>>,
    #[account(mut, seeds = [CONTEST_TOKEN_SEED, contest.id.as_ref()], bump = contest.token_bump)]
    pub contest_token: Box<Account<'info, TokenAccount>>,
    #[account(mut, address = config.treasury_token)]
    pub treasury_token: Box<Account<'info, TokenAccount>>,
    /// CHECK: rent goes back to whoever paid it.
    #[account(mut, address = contest.rent_payer)]
    pub rent_receiver: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

/// Once every entry and backing is claimed, sweeps rounding dust to the treasury and
/// returns the rent. A placement book (first remaining account, writable) is closed with it.
pub fn close_contest(ctx: Context<CloseContest>) -> Result<()> {
    let contest = &ctx.accounts.contest;
    require!(contest.status.is_terminal(), WagersError::NotFinished);
    require!(
        contest.open_entries == 0 && contest.open_backings == 0,
        WagersError::ContestNotDrained
    );
    let has_book = contest.status == ContestStatus::Settled && contest.outcome_kind == OUTCOME_BOOK;
    if has_book {
        read_book(ctx.remaining_accounts, &contest.key())?;
        require!(ctx.remaining_accounts[0].is_writable, WagersError::MissingPlacementBook);
    }
    let swept = ctx.accounts.contest_token.amount;
    let (id, bump) = (contest.id, [contest.bump]);
    let seeds = contest_seeds(&id, &bump);
    token_util::transfer(
        ctx.accounts.contest_token.to_account_info(),
        ctx.accounts.treasury_token.to_account_info(),
        contest.to_account_info(),
        ctx.accounts.usdc_mint.to_account_info(),
        ctx.accounts.usdc_mint.decimals,
        swept,
        Some(&[&seeds]),
    )?;
    token_util::close_token_account(
        ctx.accounts.contest_token.to_account_info(),
        ctx.accounts.rent_receiver.to_account_info(),
        contest.to_account_info(),
        &[&seeds],
    )?;
    if has_book {
        // After the CPIs: the runtime checks lamport balances at every CPI boundary.
        let book = &ctx.remaining_accounts[0];
        let receiver = ctx.accounts.rent_receiver.to_account_info();
        let total = receiver.lamports().checked_add(book.lamports()).ok_or(WagersError::MathOverflow)?;
        **book.try_borrow_mut_lamports()? = 0;
        **receiver.try_borrow_mut_lamports()? = total;
        book.try_borrow_mut_data()?.fill(0);
    }
    emit!(ContestClosed { contest: contest.key(), swept });
    Ok(())
}
