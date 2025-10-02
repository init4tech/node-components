use alloy::primitives::Address;
use signet_constants::SignetSystemConstants;

/// The default test constants for the Signet system.
pub const TEST_CONSTANTS: SignetSystemConstants = SignetSystemConstants::test();

/// Default reward address used in tests when no other is specified.
pub const DEFAULT_REWARD_ADDRESS: Address = Address::repeat_byte(7);
