use alloy::primitives::hex;
use reth::providers::BlockReader;
use serial_test::serial;
use signet_node::SignetNodeBuilder;
use signet_node_config::test_utils::test_config;
use signet_node_tests::utils::create_test_provider_factory_with_chain_spec;
use std::sync::Arc;

#[serial]
#[tokio::test]
async fn test_genesis() {
    let cfg = test_config();
    let consts = cfg.constants();
    let (ctx, _) = reth_exex_test_utils::test_exex_context().await.unwrap();

    let chain_spec: Arc<_> = cfg.chain_spec().clone();
    assert_eq!(chain_spec.genesis().config.chain_id, consts.unwrap().ru_chain_id());

    let factory = create_test_provider_factory_with_chain_spec(chain_spec.clone());
    let (_, _) = SignetNodeBuilder::new(cfg.clone())
        .with_ctx(ctx)
        .with_factory(factory.clone())
        .build()
        .unwrap();

    let genesis_block = factory.provider().unwrap().block_by_number(0).unwrap().unwrap();

    let want_hash = hex!("0x0000000000000000000000000000000000000000000000000000000000000000");
    assert_eq!(genesis_block.parent_hash, want_hash);
}
