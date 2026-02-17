use alloy::genesis::Genesis;
use reth::{
    chainspec::ChainSpec,
    providers::{
        ProviderFactory,
        providers::{RocksDBProvider, StaticFileProvider},
    },
};
use reth_db::test_utils::{
    create_test_rocksdb_dir, create_test_rw_db, create_test_static_files_dir,
};
use reth_exex_test_utils::TmpDB as TmpDb;
use signet_node_types::SignetNodeTypes;
use std::sync::{Arc, OnceLock};

static GENESIS_JSON: &str = include_str!("../../../../tests/artifacts/local.genesis.json");

static SPEC: OnceLock<Arc<ChainSpec>> = OnceLock::new();

/// Returns a chain spec for tests.
pub fn chain_spec() -> Arc<ChainSpec> {
    SPEC.get_or_init(|| {
        let genesis: Genesis = serde_json::from_str(GENESIS_JSON).expect("valid genesis json");
        Arc::new(genesis.into())
    })
    .clone()
}

/// Create a provider factory with a chain spec
pub fn create_test_provider_factory() -> ProviderFactory<SignetNodeTypes<TmpDb>> {
    let db = create_test_rw_db();
    let (static_dir, _) = create_test_static_files_dir();
    let (rocksdb_dir, _) = create_test_rocksdb_dir();

    let sfp = StaticFileProvider::read_write(static_dir.keep()).expect("static file provider");
    let rocks_db = RocksDBProvider::builder(rocksdb_dir.keep()).build().unwrap();

    ProviderFactory::new(db, chain_spec(), sfp, rocks_db, Default::default())
        .expect("provider factory")
}
