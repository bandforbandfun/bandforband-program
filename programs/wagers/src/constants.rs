pub const CONFIG_SEED: &[u8] = b"config";
pub const REWARDS_SEED: &[u8] = b"rewards";
pub const VAULT_SEED: &[u8] = b"vault";
pub const VAULT_TOKEN_SEED: &[u8] = b"vault_token";
pub const CONTEST_SEED: &[u8] = b"contest";
pub const CONTEST_TOKEN_SEED: &[u8] = b"contest_token";
pub const ENTRY_SEED: &[u8] = b"entry";
pub const BACKING_SEED: &[u8] = b"backing";
pub const ORDER_SEED: &[u8] = b"order";
pub const PLACEMENTS_SEED: &[u8] = b"placements";

/// Hard cap for any fee the program will ever take, in bps (10%).
pub const MAX_FEE_BPS: u16 = 1_000;
pub const MAX_FEE_TIERS: usize = 8;
/// Maximum sides in a contest (lobby players or duel teams).
pub const MAX_SIDES: usize = 64;
/// Maximum entrants (tournaments).
pub const MAX_CAPACITY: u16 = 256;
/// Maximum paid places for placement outcomes (tournaments, lobby top-N).
pub const MAX_PLACES: usize = 8;
/// Maximum paid seats of a tournament settled into a `PlacementBook`. Bounded by the
/// transaction size: each seat costs 4 bytes of instruction data.
pub const MAX_BOOK_PLACES: usize = 128;
pub const MAX_DURATION_SECS: i64 = 31 * 24 * 60 * 60;
pub const MAX_JOIN_WINDOW_SECS: i64 = 8 * 24 * 60 * 60;
pub const MAX_PREPARATION_SECS: i64 = 60 * 60;
pub const MIN_REFUND_TIMEOUT_SECS: i64 = 24 * 60 * 60;
pub const ORDER_RETENTION_SECS: i64 = 30 * 24 * 60 * 60;

/// Layout version written into every account. Bump it when an upgrade changes what an
/// account's bytes mean, and teach `migrate_account` how to bring older ones forward.
pub const ACCOUNT_VERSION: u8 = 1;
/// Zeroed bytes at the end of each account. New fields take their space from here, so the
/// account size, and every existing offset, stays the same across upgrades.
pub const RESERVED_LONG: usize = 64;
pub const RESERVED_SHORT: usize = 32;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every live account's address is derived from these. Changing one orphans them all.
    #[test]
    fn seeds_never_change() {
        let seeds = [
            CONFIG_SEED,
            REWARDS_SEED,
            VAULT_SEED,
            VAULT_TOKEN_SEED,
            CONTEST_SEED,
            CONTEST_TOKEN_SEED,
            ENTRY_SEED,
            BACKING_SEED,
            ORDER_SEED,
            PLACEMENTS_SEED,
        ];
        let expected: [&[u8]; 10] = [
            b"config",
            b"rewards",
            b"vault",
            b"vault_token",
            b"contest",
            b"contest_token",
            b"entry",
            b"backing",
            b"order",
            b"placements",
        ];
        assert_eq!(seeds, expected);
    }
}
