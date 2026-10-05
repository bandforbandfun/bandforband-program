use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Token, TransferChecked};

/// SPL `transfer_checked`, optionally signed by a PDA. Zero amounts are a no-op.
pub fn transfer<'info>(
    from: AccountInfo<'info>,
    to: AccountInfo<'info>,
    authority: AccountInfo<'info>,
    mint: AccountInfo<'info>,
    decimals: u8,
    amount: u64,
    signer_seeds: Option<&[&[&[u8]]]>,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let accounts = TransferChecked { from, mint, to, authority };
    let ctx = match signer_seeds {
        Some(seeds) => CpiContext::new_with_signer(Token::id(), accounts, seeds),
        None => CpiContext::new(Token::id(), accounts),
    };
    token::transfer_checked(ctx, amount, decimals)
}

pub fn close_token_account<'info>(
    account: AccountInfo<'info>,
    destination: AccountInfo<'info>,
    authority: AccountInfo<'info>,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let accounts = CloseAccount { account, destination, authority };
    token::close_account(CpiContext::new_with_signer(Token::id(), accounts, signer_seeds))
}
