use anchor_lang::prelude::*;

#[error_code]
pub enum WagersError {
    #[msg("Program is paused for new contests, backings and purchases")]
    Paused,
    #[msg("Only the config admin can do this")]
    NotAdmin,
    #[msg("Only the settler can do this")]
    NotSettler,
    #[msg("Invalid config parameters")]
    InvalidConfig,
    #[msg("Invalid amount")]
    InvalidAmount,
    #[msg("Fee above the allowed cap")]
    FeeTooHigh,
    #[msg("Invalid contest parameters")]
    InvalidContestParams,
    #[msg("Contest is not open")]
    ContestNotOpen,
    #[msg("Contest join deadline has passed")]
    JoinDeadlinePassed,
    #[msg("Contest is full")]
    ContestFull,
    #[msg("Invalid side")]
    InvalidSide,
    #[msg("This side is full")]
    SideFull,
    #[msg("Contest is not locked")]
    ContestNotLocked,
    #[msg("Contest has not started yet")]
    NotStarted,
    #[msg("Contest cannot be cancelled now")]
    CannotCancel,
    #[msg("Contest is not finished")]
    NotFinished,
    #[msg("Invalid outcome")]
    InvalidOutcome,
    #[msg("Entry or backing already settled")]
    AlreadySettled,
    #[msg("Not enough entrants to start")]
    NotEnoughEntrants,
    #[msg("Backing is disabled")]
    BackingDisabled,
    #[msg("Backing is closed: the contest has started")]
    BackingClosed,
    #[msg("Players cannot back a contest they are in")]
    PlayerCannotBack,
    #[msg("A backing can only add to the same side")]
    BackingSideMismatch,
    #[msg("Backing above the per-user limit")]
    BackingLimit,
    #[msg("Contest still has unclaimed entries or backings")]
    ContestNotDrained,
    #[msg("Account does not belong to this contest")]
    WrongContest,
    #[msg("Cannot send to yourself")]
    SelfTransfer,
    #[msg("Order is too recent to close")]
    OrderTooRecent,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("No pending admin")]
    NoPendingAdmin,
    #[msg("Not a program account that can be migrated")]
    NotMigratable,
    #[msg("Account was written by a newer program version")]
    UnknownAccountVersion,
    #[msg("Vault still holds funds")]
    VaultNotEmpty,
    #[msg("Vault predates rent tracking and cannot be closed")]
    VaultNotClosable,
    #[msg("The contest's placement book must be passed")]
    MissingPlacementBook,
}
