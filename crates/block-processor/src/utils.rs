use reth_chainspec::EthereumHardforks;
use trevm::revm::primitives::hardfork::SpecId;

/// Equivalent to `reth_evm_ethereum::revm_spec`, however, always starts at
/// [`SpecId::PRAGUE`] and transitions to [`SpecId::OSAKA`].
pub fn revm_spec(chain_spec: &reth::chainspec::ChainSpec, timestamp: u64) -> SpecId {
    if chain_spec.is_amsterdam_active_at_timestamp(timestamp) {
        SpecId::AMSTERDAM
    } else if chain_spec.is_osaka_active_at_timestamp(timestamp) {
        SpecId::OSAKA
    } else {
        SpecId::PRAGUE
    }
}

/// This is simply a compile-time assertion to ensure that all SpecIds are
/// covered in the match. When this fails to compile, it indicates that a new
/// hardfork has been added and [`revm_spec`] needs to be updated.
#[allow(dead_code)]
const fn assert_in_range(spec_id: SpecId) {
    match spec_id {
        SpecId::FRONTIER
        | SpecId::FRONTIER_THAWING
        | SpecId::HOMESTEAD
        | SpecId::DAO_FORK
        | SpecId::TANGERINE
        | SpecId::SPURIOUS_DRAGON
        | SpecId::BYZANTIUM
        | SpecId::CONSTANTINOPLE
        | SpecId::PETERSBURG
        | SpecId::ISTANBUL
        | SpecId::MUIR_GLACIER
        | SpecId::BERLIN
        | SpecId::LONDON
        | SpecId::ARROW_GLACIER
        | SpecId::GRAY_GLACIER
        | SpecId::MERGE
        | SpecId::SHANGHAI
        | SpecId::CANCUN
        | SpecId::PRAGUE
        | SpecId::OSAKA
        | SpecId::AMSTERDAM => {}
    }
}
