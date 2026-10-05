//! Pure settlement math. No Solana types, so every rule here is unit and property
//! tested on the host (`cargo test --lib`). The instruction handlers only call into
//! these functions, which keeps the money logic in one audited place.

pub const BPS_DENOMINATOR: u64 = 10_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tier {
    pub min_stake: u64,
    pub bps: u16,
}

/// Returns `amount * bps / 10_000` rounded down, or None on invalid bps.
pub fn mul_bps(amount: u64, bps: u16) -> Option<u64> {
    if bps as u64 > BPS_DENOMINATOR {
        return None;
    }
    Some(((amount as u128 * bps as u128) / BPS_DENOMINATOR as u128) as u64)
}

/// Rake tier for a per-seat stake: the last tier whose `min_stake <= stake`.
pub fn tier_bps(tiers: &[Tier], stake: u64) -> u16 {
    let mut bps = tiers.first().map(|t| t.bps).unwrap_or(0);
    for tier in tiers {
        if stake >= tier.min_stake {
            bps = tier.bps;
        }
    }
    bps
}

/// Tiers must start at 0, strictly ascend by stake, never increase in bps and never
/// exceed `max_bps`.
pub fn validate_tiers(tiers: &[Tier], max_bps: u16) -> bool {
    if tiers.is_empty() || tiers[0].min_stake != 0 {
        return false;
    }
    for (i, tier) in tiers.iter().enumerate() {
        if tier.bps > max_bps {
            return false;
        }
        if i > 0 && (tier.min_stake <= tiers[i - 1].min_stake || tier.bps > tiers[i - 1].bps) {
            return false;
        }
    }
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SidesSettlement {
    pub fee: u64,
    /// Paid to every seat on a winning side.
    pub per_seat: u64,
    /// Rounding remainder, sent to the treasury with the fee.
    pub dust: u64,
}

/// Winner-side settlement: the pot minus the fee is shared equally between all seats
/// on the winning side(s).
pub fn settle_sides(pot: u64, fee_bps: u16, winning_seats: u64) -> Option<SidesSettlement> {
    if winning_seats == 0 {
        return None;
    }
    let fee = mul_bps(pot, fee_bps)?;
    let net = pot.checked_sub(fee)?;
    let per_seat = net / winning_seats;
    let dust = net.checked_sub(per_seat.checked_mul(winning_seats)?)?;
    Some(SidesSettlement { fee, per_seat, dust })
}

/// Placement payout for the i-th paid place. Rounding dust goes to first place so the
/// sum of all places equals `net` exactly.
pub fn placement_payout(net: u64, bps: &[u16], place: usize) -> Option<u64> {
    if place >= bps.len() {
        return None;
    }
    let share = |b: u16| ((net as u128 * b as u128) / BPS_DENOMINATOR as u128) as u64;
    if place == 0 {
        let others: u64 = bps.iter().skip(1).map(|b| share(*b)).sum();
        net.checked_sub(others)
    } else {
        Some(share(bps[place]))
    }
}

pub fn validate_placements(bps: &[u16], max_places: usize) -> bool {
    if bps.is_empty() || bps.len() > max_places {
        return false;
    }
    bps.iter().map(|b| *b as u64).sum::<u64>() == BPS_DENOMINATOR && bps.iter().all(|b| *b > 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackingSettlement {
    pub refund_all: bool,
    pub fee: u64,
    pub winning_pool: u64,
    /// Losing pool minus the fee, shared pro-rata between winning backers.
    pub net_losing: u64,
}

/// Parimutuel backing settlement. When one side has no backers (or nobody won) every
/// backer is refunded in full and no fee is taken.
pub fn settle_backing(pools: &[u64], winning_mask: u64, fee_bps: u16) -> Option<BackingSettlement> {
    let mut winning_pool: u64 = 0;
    let mut total: u64 = 0;
    for (side, pool) in pools.iter().enumerate() {
        total = total.checked_add(*pool)?;
        if side < 64 && winning_mask & (1u64 << side) != 0 {
            winning_pool = winning_pool.checked_add(*pool)?;
        }
    }
    let losing = total.checked_sub(winning_pool)?;
    if winning_pool == 0 || losing == 0 {
        return Some(BackingSettlement { refund_all: true, fee: 0, winning_pool, net_losing: 0 });
    }
    let fee = mul_bps(losing, fee_bps)?;
    Some(BackingSettlement { refund_all: false, fee, winning_pool, net_losing: losing - fee })
}

/// What a single backer receives. Losers receive 0, winners their stake plus their
/// pro-rata share of the net losing pool (rounded down, dust stays for the treasury).
pub fn backing_payout(settlement: &BackingSettlement, amount: u64, is_winner: bool) -> Option<u64> {
    if settlement.refund_all {
        return Some(amount);
    }
    if !is_winner {
        return Some(0);
    }
    let bonus = (amount as u128)
        .checked_mul(settlement.net_losing as u128)?
        .checked_div(settlement.winning_pool as u128)?;
    amount.checked_add(u64::try_from(bonus).ok()?)
}

/// Seats occupied on the winning sides.
pub fn winning_seats(side_counts: &[u16], sides: u16, mask: u64) -> Option<u64> {
    if mask == 0 || sides == 0 || sides > 64 {
        return None;
    }
    if sides < 64 && mask >> sides != 0 {
        return None;
    }
    let mut seats: u64 = 0;
    for side in 0..sides as usize {
        if mask & (1u64 << side) != 0 {
            let count = *side_counts.get(side)? as u64;
            if count == 0 {
                return None;
            }
            seats += count;
        }
    }
    Some(seats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn tiers() -> Vec<Tier> {
        vec![
            Tier { min_stake: 0, bps: 500 },
            Tier { min_stake: 10_000_000, bps: 400 },
            Tier { min_stake: 50_000_000, bps: 300 },
            Tier { min_stake: 100_000_000, bps: 200 },
            Tier { min_stake: 500_000_000, bps: 100 },
        ]
    }

    #[test]
    fn tier_lookup_matches_app_config() {
        let t = tiers();
        assert_eq!(tier_bps(&t, 1_000_000), 500);
        assert_eq!(tier_bps(&t, 9_999_999), 500);
        assert_eq!(tier_bps(&t, 10_000_000), 400);
        assert_eq!(tier_bps(&t, 10_000_000_000), 100);
        assert!(validate_tiers(&t, 1_000));
    }

    #[test]
    fn rejects_bad_tiers() {
        assert!(!validate_tiers(&[], 1_000));
        assert!(!validate_tiers(&[Tier { min_stake: 1, bps: 100 }], 1_000));
        assert!(!validate_tiers(&[Tier { min_stake: 0, bps: 1_500 }], 1_000));
        let increasing = [Tier { min_stake: 0, bps: 100 }, Tier { min_stake: 10, bps: 200 }];
        assert!(!validate_tiers(&increasing, 1_000));
        let unsorted = [Tier { min_stake: 0, bps: 300 }, Tier { min_stake: 0, bps: 200 }];
        assert!(!validate_tiers(&unsorted, 1_000));
    }

    #[test]
    fn one_v_one_example() {
        // Two $10 stakes, 4% rake: fee 0.80, winner gets 19.20.
        let s = settle_sides(20_000_000, 400, 1).unwrap();
        assert_eq!(s.fee, 800_000);
        assert_eq!(s.per_seat, 19_200_000);
        assert_eq!(s.dust, 0);
    }

    #[test]
    fn team_split_has_dust_to_treasury() {
        let s = settle_sides(10_000_001, 0, 3).unwrap();
        assert_eq!(s.per_seat, 3_333_333);
        assert_eq!(s.dust, 2);
        assert!(settle_sides(1, 0, 0).is_none());
    }

    #[test]
    fn winning_seats_validation() {
        let counts = [1u16, 1, 0, 0];
        assert_eq!(winning_seats(&counts, 2, 0b01), Some(1));
        assert_eq!(winning_seats(&counts, 2, 0b11), Some(2));
        assert_eq!(winning_seats(&counts, 2, 0), None);
        assert_eq!(winning_seats(&counts, 2, 0b100), None, "bit outside sides");
        assert_eq!(winning_seats(&[1, 0], 2, 0b10), None, "empty winning side");
    }

    #[test]
    fn backing_refunds_when_one_side_is_empty() {
        let s = settle_backing(&[100, 0], 0b01, 300).unwrap();
        assert!(s.refund_all);
        assert_eq!(backing_payout(&s, 100, true), Some(100));
        let s = settle_backing(&[0, 100], 0b01, 300).unwrap();
        assert!(s.refund_all);
    }

    #[test]
    fn backing_example() {
        // 300 on side 0 (winner), 100 on side 1, 3% fee on the losing pool.
        let s = settle_backing(&[300, 100], 0b01, 300).unwrap();
        assert_eq!(s.fee, 3);
        assert_eq!(s.net_losing, 97);
        assert_eq!(backing_payout(&s, 150, true), Some(150 + 48));
        assert_eq!(backing_payout(&s, 100, false), Some(0));
    }

    proptest! {
        #[test]
        fn sides_conserve_the_pot(pot in 0u64..=u64::MAX / 2, fee_bps in 0u16..=1_000, seats in 1u64..=64) {
            let s = settle_sides(pot, fee_bps, seats).unwrap();
            prop_assert_eq!(s.fee + s.per_seat * seats + s.dust, pot);
            prop_assert!(s.dust < seats);
        }

        #[test]
        fn placements_conserve_net(net in 0u64..=u64::MAX / 2, raw in proptest::collection::vec(1u16..=5_000, 1..=8)) {
            let total: u64 = raw.iter().map(|b| *b as u64).sum();
            let mut bps: Vec<u16> = raw.iter().map(|b| ((*b as u64 * 10_000) / total) as u16).collect();
            let assigned: u64 = bps.iter().map(|b| *b as u64).sum();
            bps[0] += (10_000 - assigned) as u16;
            prop_assume!(bps.iter().all(|b| *b > 0));
            prop_assert!(validate_placements(&bps, 8));
            let sum: u64 = (0..bps.len()).map(|i| placement_payout(net, &bps, i).unwrap()).sum();
            prop_assert_eq!(sum, net);
        }

        #[test]
        fn book_places_conserve_net(net in 0u64..=u64::MAX / 2, raw in proptest::collection::vec(0u16..=60, 1..=128)) {
            let mut bps: Vec<u16> = raw.iter().map(|b| b + 1).collect();
            let assigned: u64 = bps.iter().map(|b| *b as u64).sum();
            bps[0] += (10_000 - assigned) as u16;
            prop_assert!(validate_placements(&bps, 128));
            prop_assert!(!validate_placements(&bps, bps.len() - 1));
            let sum: u64 = (0..bps.len()).map(|i| placement_payout(net, &bps, i).unwrap()).sum();
            prop_assert_eq!(sum, net);
        }

        #[test]
        fn backing_never_pays_more_than_the_pools(
            pools in proptest::collection::vec(0u64..=1_000_000_000_000, 2..=8),
            mask_seed in 1u64..=255,
            fee_bps in 0u16..=1_000,
            split in 1u64..=20,
        ) {
            let sides = pools.len();
            let mask = mask_seed & ((1u64 << sides) - 1);
            prop_assume!(mask != 0);
            let s = settle_backing(&pools, mask, fee_bps).unwrap();
            let total: u64 = pools.iter().sum();
            // Simulate each pool being split between `split` equal backers (+ remainder).
            let mut paid: u64 = 0;
            for (side, pool) in pools.iter().enumerate() {
                let is_winner = mask & (1 << side) != 0;
                let each = pool / split;
                let rest = pool - each * split;
                for i in 0..split {
                    let amount = if i == 0 { each + rest } else { each };
                    paid += backing_payout(&s, amount, is_winner).unwrap();
                }
            }
            prop_assert!(paid + s.fee <= total, "paid {} + fee {} > total {}", paid, s.fee, total);
            if s.refund_all {
                prop_assert_eq!(paid, total);
                prop_assert_eq!(s.fee, 0);
            }
        }

        #[test]
        fn tier_is_monotonic(a in 0u64..=u64::MAX, b in 0u64..=u64::MAX) {
            let t = tiers();
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(tier_bps(&t, hi) <= tier_bps(&t, lo));
        }
    }
}
