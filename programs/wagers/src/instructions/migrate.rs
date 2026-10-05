use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};
use anchor_lang::Discriminator;

use crate::constants::ACCOUNT_VERSION;
use crate::errors::WagersError;
use crate::events::AccountMigrated;
use crate::state::*;

/// Brings one program account up to the layout this build expects: grows it to the current
/// size (new bytes are zero) and moves its `version` forward.
///
/// Permissionless on purpose. A migration may only derive the new layout from the account's
/// own bytes, so whoever runs it can't influence the result; they only pay the extra rent.
/// Anything that needs a decision belongs in an admin instruction instead.
#[derive(Accounts)]
pub struct MigrateAccount<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: any account owned by this program. Its type comes from its discriminator.
    #[account(mut, owner = crate::ID @ WagersError::NotMigratable)]
    pub target: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

pub fn migrate_account(ctx: Context<MigrateAccount>) -> Result<()> {
    let discriminator: [u8; 8] = {
        let data = ctx.accounts.target.try_borrow_data()?;
        data.get(..8)
            .and_then(|d| d.try_into().ok())
            .ok_or(WagersError::NotMigratable)?
    };
    let d = &discriminator[..];
    if d == Config::DISCRIMINATOR {
        migrate::<Config>(&ctx.accounts, 8 + Config::INIT_SPACE)
    } else if d == UserVault::DISCRIMINATOR {
        migrate::<UserVault>(&ctx.accounts, 8 + UserVault::INIT_SPACE)
    } else if d == Contest::DISCRIMINATOR {
        migrate::<Contest>(&ctx.accounts, 8 + Contest::INIT_SPACE)
    } else if d == Entry::DISCRIMINATOR {
        migrate::<Entry>(&ctx.accounts, 8 + Entry::INIT_SPACE)
    } else if d == Backing::DISCRIMINATOR {
        migrate::<Backing>(&ctx.accounts, 8 + Backing::INIT_SPACE)
    } else if d == Order::DISCRIMINATOR {
        migrate::<Order>(&ctx.accounts, 8 + Order::INIT_SPACE)
    } else if d == PlacementBook::DISCRIMINATOR {
        migrate::<PlacementBook>(&ctx.accounts, 8 + PlacementBook::INIT_SPACE)
    } else if d == Mandate::DISCRIMINATOR {
        migrate::<Mandate>(&ctx.accounts, 8 + Mandate::INIT_SPACE)
    } else {
        err!(WagersError::NotMigratable)
    }
}

fn migrate<T>(accounts: &MigrateAccount, space: usize) -> Result<()>
where
    T: AccountSerialize + AccountDeserialize + Versioned,
{
    let target = accounts.target.to_account_info();
    let from_size = target.data_len();
    if from_size < space {
        let missing = Rent::get()?.minimum_balance(space).saturating_sub(target.lamports());
        if missing > 0 {
            let transfer = Transfer { from: accounts.payer.to_account_info(), to: target.clone() };
            system_program::transfer(CpiContext::new(System::id(), transfer), missing)?;
        }
        target.resize(space)?;
    }

    let mut account = Box::new(T::try_deserialize(&mut &target.try_borrow_data()?[..])?);
    let from_version = account.version();
    require!(from_version <= ACCOUNT_VERSION, WagersError::UnknownAccountVersion);
    if from_version == ACCOUNT_VERSION && from_size >= space {
        return Ok(());
    }

    // Version 0 is the launch layout without `version` and `reserved`: growing the account
    // appended both, zeroed, so there is nothing to rewrite. Later versions add their
    // conversions here, one step at a time.
    account.set_version(ACCOUNT_VERSION);
    {
        let mut data = target.try_borrow_mut_data()?;
        let mut writer: &mut [u8] = &mut data[..];
        account.try_serialize(&mut writer)?;
    }

    emit!(AccountMigrated {
        account: target.key(),
        from_size: from_size as u32,
        to_size: target.data_len() as u32,
        from_version,
        to_version: ACCOUNT_VERSION,
    });
    Ok(())
}
