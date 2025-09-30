use alloy::{
    network::{Ethereum, EthereumWallet},
    providers::{
        Identity, RootProvider,
        fillers::{
            BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller,
            SimpleNonceManager, WalletFiller,
        },
    },
};

/// The alloy provider type for the Signet Node test context
pub type CtxProvider = FillProvider<
    JoinFill<
        JoinFill<
            JoinFill<
                JoinFill<JoinFill<Identity, BlobGasFiller>, GasFiller>,
                NonceFiller<SimpleNonceManager>,
            >,
            ChainIdFiller,
        >,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider,
>;

/// Counter contract instance type with the Signet test context's provider.
pub type TestCounterInstance = Counter::CounterInstance<CtxProvider, Ethereum>;

/// Log contract instance type with the Signet test context's provider.
pub type TestLogInstance = Log::LogInstance<CtxProvider, Ethereum>;

/// ERC20 contract instance type with the Signet test context's provider.
pub type TestErc20Instance = Erc20::Erc20Instance<CtxProvider, Ethereum>;

pub use signet_test_utils::contracts::counter::Counter;

alloy::sol! {
    #[derive(Debug)]
    #[sol(rpc)]
    contract Erc20 {
        function balanceOf(address) public view returns (uint256);
        function approve(address, uint256) public returns (bool);

        function name() public view returns (string);
        function symbol() public view returns (string);
        function decimals() public view returns (uint8);
    }
}

// Codegen from embedded Solidity code and precompiled bytecode.
// solc v0.8.25 Log.sol --via-ir --optimize --bin
alloy::sol!(
    #[allow(missing_docs)]
    #[sol(rpc, bytecode = "6080806040523460135760c9908160188239f35b5f80fdfe6004361015600b575f80fd5b5f3560e01c80637b3ab2d014605f57639ee1a440146027575f80fd5b34605b575f366003190112605b577f2d67bb91f17bca05af6764ab411e86f4ddf757adb89fcec59a7d21c525d417125f80a1005b5f80fd5b34605b575f366003190112605b577fbcdfe0d5b27dd186282e187525415c57ea3077c34efb39148111e4d342e7ab0e5f80a100fea2646970667358221220f6b42b522bc9fb2b4c7d7e611c7c3e995d057ecab7fd7be4179712804c886b4f64736f6c63430008190033")]
    contract Log {
        #[derive(Debug)]
        event Hello();
        event World();

        function emitHello() public {
            emit Hello();
        }

        function emitWorld() public {
            emit World();
        }
    }
);
