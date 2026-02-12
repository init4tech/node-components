//! Integration tests for the `signet-rpc-storage` ETH RPC endpoints.
//!
//! Tests exercise the public router API via the axum service layer, using
//! in-memory storage backends (`MemKv` + `MemColdBackend`).

use alloy::{
    consensus::{
        EthereumTxEnvelope, Header, Receipt as AlloyReceipt, SignableTransaction, Signed, TxLegacy,
        TxType,
    },
    primitives::{Address, B256, Log as PrimitiveLog, LogData, TxKind, U256, address, logs_bloom},
};
use axum::body::Body;
use http::Request;
use serde_json::{Value, json};
use signet_cold::{BlockData, ColdStorageHandle, ColdStorageTask, mem::MemColdBackend};
use signet_constants::SignetSystemConstants;
use signet_hot::{HotKv, db::UnsafeDbWrite, mem::MemKv};
use signet_rpc_storage::{BlockTags, NewBlockNotification, StorageRpcConfig, StorageRpcCtx};
use signet_storage::UnifiedStorage;
use signet_storage_types::Receipt;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Everything needed to make RPC calls against the storage-backed router.
struct TestHarness {
    app: axum::Router,
    cold: ColdStorageHandle,
    hot: MemKv,
    tags: BlockTags,
    #[allow(dead_code)]
    notif_tx: broadcast::Sender<NewBlockNotification>,
    ctx: StorageRpcCtx<MemKv>,
    _cancel: CancellationToken,
}

impl TestHarness {
    /// Create a minimal harness with empty storage.
    async fn new(latest: u64) -> Self {
        let cancel = CancellationToken::new();
        let hot = MemKv::new();
        let cold = ColdStorageTask::spawn(MemColdBackend::new(), cancel.clone());
        let storage = UnifiedStorage::new(hot.clone(), cold.clone());
        let constants = SignetSystemConstants::test();
        let tags = BlockTags::new(latest, latest.saturating_sub(2), 0);
        let (notif_tx, _) = broadcast::channel::<NewBlockNotification>(16);
        let ctx = StorageRpcCtx::new(
            storage,
            constants,
            tags.clone(),
            None,
            StorageRpcConfig::default(),
            notif_tx.clone(),
        );
        let app = signet_rpc_storage::eth::<MemKv>().into_axum("/").with_state(ctx.clone());

        Self { app, cold, hot, tags, notif_tx, ctx, _cancel: cancel }
    }

    /// Build an axum router for the debug namespace.
    fn debug_app(&self) -> axum::Router {
        signet_rpc_storage::debug::<MemKv>().into_axum("/").with_state(self.ctx.clone())
    }

    /// Build an axum router for the signet namespace.
    #[allow(dead_code)]
    fn signet_app(&self) -> axum::Router {
        signet_rpc_storage::signet::<MemKv>().into_axum("/").with_state(self.ctx.clone())
    }
}

/// Make a JSON-RPC call and return the `"result"` field.
///
/// The `method` parameter is the short name (e.g. `"blockNumber"`), without
/// the `eth_` prefix. The router registers methods without namespace prefix.
///
/// Panics if the response contains an `"error"` field.
async fn rpc_call(app: &axum::Router, method: &str, params: Value) -> Value {
    let resp = rpc_call_raw(app, method, params).await;
    if let Some(error) = resp.get("error") {
        panic!("RPC error for {method}: {error}");
    }
    resp["result"].clone()
}

/// Make a JSON-RPC call and return the full response (including any error).
async fn rpc_call_raw(app: &axum::Router, method: &str, params: Value) -> Value {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });

    let req = Request::builder()
        .method("POST")
        .uri("/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// ---------------------------------------------------------------------------
// Test data builders
// ---------------------------------------------------------------------------

/// Test address used for account state queries.
const TEST_ADDR: Address = address!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

/// Test log-emitting contract address.
const LOG_ADDR: Address = address!("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

/// Test log topic.
const LOG_TOPIC: B256 = B256::repeat_byte(0xcc);

/// Create a legacy transaction signed with a deterministic key.
///
/// Uses alloy's signer to produce a valid ECDSA signature so that
/// `recover_sender` succeeds during RPC response building.
fn make_signed_tx(nonce: u64) -> (signet_storage_types::TransactionSigned, Address) {
    make_signed_tx_with_gas_price(nonce, 1_000_000_000)
}

/// Create a legacy transaction with a custom gas price.
fn make_signed_tx_with_gas_price(
    nonce: u64,
    gas_price: u128,
) -> (signet_storage_types::TransactionSigned, Address) {
    use alloy::signers::{SignerSync, local::PrivateKeySigner};

    let signer = PrivateKeySigner::from_signing_key(
        alloy::signers::k256::ecdsa::SigningKey::from_slice(
            &B256::repeat_byte((nonce as u8).wrapping_add(1)).0,
        )
        .unwrap(),
    );
    let sender = signer.address();

    let tx = TxLegacy {
        nonce,
        gas_price,
        gas_limit: 21_000,
        to: TxKind::Call(Address::ZERO),
        value: U256::from(1000),
        ..Default::default()
    };

    let sig_hash = tx.signature_hash();
    let sig = signer.sign_hash_sync(&sig_hash).unwrap();
    let signed: signet_storage_types::TransactionSigned =
        EthereumTxEnvelope::Legacy(Signed::new_unhashed(tx, sig));

    (signed, sender)
}

/// Build a [`BlockData`] from pre-signed transactions.
///
/// Creates receipts with incrementing `cumulative_gas_used` and optionally
/// attaches logs to each receipt.
fn make_block(
    block_num: u64,
    txs: Vec<signet_storage_types::TransactionSigned>,
    logs_per_receipt: usize,
) -> BlockData {
    let receipts: Vec<Receipt> = txs
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let logs: Vec<PrimitiveLog> = (0..logs_per_receipt)
                .map(|l| PrimitiveLog {
                    address: LOG_ADDR,
                    data: LogData::new_unchecked(
                        vec![LOG_TOPIC],
                        alloy::primitives::Bytes::from(vec![l as u8]),
                    ),
                })
                .collect();

            Receipt {
                tx_type: TxType::Legacy,
                inner: AlloyReceipt {
                    status: true.into(),
                    cumulative_gas_used: 21_000 * (i as u64 + 1),
                    logs,
                },
            }
        })
        .collect();

    // Compute the logs bloom from all receipt logs so getLogs bloom check passes.
    let all_logs: Vec<_> = receipts.iter().flat_map(|r| r.inner.logs.iter()).collect();
    let bloom = logs_bloom(all_logs);

    let gas_used = receipts.last().map(|r| r.inner.cumulative_gas_used).unwrap_or_default();

    let header = Header {
        number: block_num,
        timestamp: 1_700_000_000 + block_num,
        base_fee_per_gas: Some(1_000_000_000),
        gas_limit: 30_000_000,
        gas_used,
        logs_bloom: bloom,
        ..Default::default()
    };

    BlockData::new(header, txs, receipts, vec![], None)
}

// ---------------------------------------------------------------------------
// Group 1: Simple queries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_block_number() {
    let h = TestHarness::new(42).await;
    let result = rpc_call(&h.app, "blockNumber", json!([])).await;
    assert_eq!(result, json!("0x2a"));
}

#[tokio::test]
async fn test_chain_id() {
    let h = TestHarness::new(0).await;
    let result = rpc_call(&h.app, "chainId", json!([])).await;
    let expected = format!("0x{:x}", SignetSystemConstants::test().ru_chain_id());
    assert_eq!(result, json!(expected));
}

// ---------------------------------------------------------------------------
// Group 2: Cold storage — block queries
// ---------------------------------------------------------------------------

/// Shared setup: append a block with 2 signed transactions to cold storage.
async fn setup_cold_block(h: &TestHarness) -> (Vec<B256>, Vec<Address>) {
    let (tx0, sender0) = make_signed_tx(0);
    let (tx1, sender1) = make_signed_tx(1);

    let hash0 = *tx0.tx_hash();
    let hash1 = *tx1.tx_hash();

    let block = make_block(1, vec![tx0, tx1], 1);
    h.cold.append_block(block).await.unwrap();
    h.tags.set_latest(1);

    (vec![hash0, hash1], vec![sender0, sender1])
}

#[tokio::test]
async fn test_get_block_by_number_hashes() {
    let h = TestHarness::new(0).await;
    let (tx_hashes, _) = setup_cold_block(&h).await;

    let result = rpc_call(&h.app, "getBlockByNumber", json!(["0x1", false])).await;

    assert_eq!(result["number"], json!("0x1"));
    let txs = result["transactions"].as_array().unwrap();
    assert_eq!(txs.len(), 2);
    // When full=false, transactions are hashes (strings)
    assert!(txs[0].is_string());
    assert_eq!(txs[0].as_str().unwrap(), format!("{:?}", tx_hashes[0]));
}

#[tokio::test]
async fn test_get_block_by_number_full() {
    let h = TestHarness::new(0).await;
    let (tx_hashes, senders) = setup_cold_block(&h).await;

    let result = rpc_call(&h.app, "getBlockByNumber", json!(["0x1", true])).await;

    assert_eq!(result["number"], json!("0x1"));
    let txs = result["transactions"].as_array().unwrap();
    assert_eq!(txs.len(), 2);
    // When full=true, transactions are objects
    assert!(txs[0].is_object());
    assert_eq!(txs[0]["hash"], json!(format!("{:?}", tx_hashes[0])));
    assert_eq!(txs[0]["from"], json!(format!("{:?}", senders[0])));
    assert_eq!(txs[0]["blockNumber"], json!("0x1"));
    assert_eq!(txs[0]["transactionIndex"], json!("0x0"));
    assert_eq!(txs[1]["transactionIndex"], json!("0x1"));
}

#[tokio::test]
async fn test_get_block_by_hash() {
    let h = TestHarness::new(0).await;
    setup_cold_block(&h).await;

    // Get the block to learn its hash
    let block = rpc_call(&h.app, "getBlockByNumber", json!(["0x1", false])).await;
    let block_hash = block["hash"].as_str().unwrap().to_string();

    let result = rpc_call(&h.app, "getBlockByHash", json!([block_hash, false])).await;
    assert_eq!(result["number"], json!("0x1"));
    assert_eq!(result["hash"], json!(block_hash));
}

#[tokio::test]
async fn test_get_block_tx_count() {
    let h = TestHarness::new(0).await;
    setup_cold_block(&h).await;

    let result = rpc_call(&h.app, "getBlockTransactionCountByNumber", json!(["0x1"])).await;
    assert_eq!(result, json!("0x2"));
}

#[tokio::test]
async fn test_get_block_header() {
    let h = TestHarness::new(0).await;
    setup_cold_block(&h).await;

    let result = rpc_call(&h.app, "getBlockHeaderByNumber", json!(["0x1"])).await;
    assert_eq!(result["number"], json!("0x1"));
    assert!(result["baseFeePerGas"].is_string());
}

#[tokio::test]
async fn test_get_block_not_found() {
    let h = TestHarness::new(255).await;
    let result = rpc_call(&h.app, "getBlockByNumber", json!(["0xff", false])).await;
    assert!(result.is_null());
}

// ---------------------------------------------------------------------------
// Group 3: Cold storage — transaction queries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_transaction_by_hash() {
    let h = TestHarness::new(0).await;
    let (tx_hashes, senders) = setup_cold_block(&h).await;

    let result =
        rpc_call(&h.app, "getTransactionByHash", json!([format!("{:?}", tx_hashes[0])])).await;

    assert_eq!(result["hash"], json!(format!("{:?}", tx_hashes[0])));
    assert_eq!(result["from"], json!(format!("{:?}", senders[0])));
    assert_eq!(result["blockNumber"], json!("0x1"));
    assert_eq!(result["transactionIndex"], json!("0x0"));
}

#[tokio::test]
async fn test_get_raw_transaction_by_hash() {
    let h = TestHarness::new(0).await;
    let (tx_hashes, _) = setup_cold_block(&h).await;

    let result =
        rpc_call(&h.app, "getRawTransactionByHash", json!([format!("{:?}", tx_hashes[0])])).await;

    // Raw transaction is a hex string
    let hex = result.as_str().unwrap();
    assert!(hex.starts_with("0x"));
    assert!(hex.len() > 4);
}

#[tokio::test]
async fn test_get_tx_by_block_and_index() {
    let h = TestHarness::new(0).await;
    let (tx_hashes, senders) = setup_cold_block(&h).await;

    let result =
        rpc_call(&h.app, "getTransactionByBlockNumberAndIndex", json!(["0x1", "0x0"])).await;

    assert_eq!(result["hash"], json!(format!("{:?}", tx_hashes[0])));
    assert_eq!(result["from"], json!(format!("{:?}", senders[0])));
}

#[tokio::test]
async fn test_get_transaction_receipt() {
    let h = TestHarness::new(0).await;
    let (tx_hashes, senders) = setup_cold_block(&h).await;

    let result =
        rpc_call(&h.app, "getTransactionReceipt", json!([format!("{:?}", tx_hashes[0])])).await;

    assert_eq!(result["transactionHash"], json!(format!("{:?}", tx_hashes[0])));
    assert_eq!(result["from"], json!(format!("{:?}", senders[0])));
    assert_eq!(result["blockNumber"], json!("0x1"));
    assert_eq!(result["status"], json!("0x1"));
    assert_eq!(result["gasUsed"], json!("0x5208")); // 21000
}

#[tokio::test]
async fn test_get_block_receipts() {
    let h = TestHarness::new(0).await;
    setup_cold_block(&h).await;

    let result = rpc_call(&h.app, "getBlockReceipts", json!(["0x1"])).await;

    let receipts = result.as_array().unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0]["transactionIndex"], json!("0x0"));
    assert_eq!(receipts[1]["transactionIndex"], json!("0x1"));
    assert_eq!(receipts[0]["status"], json!("0x1"));
    assert_eq!(receipts[1]["status"], json!("0x1"));
}

// ---------------------------------------------------------------------------
// Group 4: Hot storage — account state
// ---------------------------------------------------------------------------

/// Populate hot storage with a test account.
fn setup_hot_account(hot: &MemKv) {
    use signet_storage_types::Account;
    use trevm::revm::bytecode::Bytecode;

    let writer = hot.writer().unwrap();

    let code = alloy::primitives::Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xf3]);
    let bytecode = Bytecode::new_raw(code);
    let code_hash = bytecode.hash_slow();

    writer
        .put_account(
            &TEST_ADDR,
            &Account {
                nonce: 5,
                balance: U256::from(1_000_000_000_000_000_000u128),
                bytecode_hash: Some(code_hash),
            },
        )
        .unwrap();

    writer.put_storage(&TEST_ADDR, &U256::from(42), &U256::from(999)).unwrap();

    writer.put_bytecode(&code_hash, &bytecode).unwrap();

    writer.commit().unwrap();
}

#[tokio::test]
async fn test_get_balance() {
    let h = TestHarness::new(1).await;
    setup_hot_account(&h.hot);

    // Append a dummy block so tag resolution succeeds
    let block = make_block(1, vec![], 0);
    h.cold.append_block(block).await.unwrap();

    let result =
        rpc_call(&h.app, "getBalance", json!([format!("{:?}", TEST_ADDR), "latest"])).await;

    // 1 ETH = 10^18
    assert_eq!(result, json!("0xde0b6b3a7640000"));
}

#[tokio::test]
async fn test_get_transaction_count() {
    let h = TestHarness::new(1).await;
    setup_hot_account(&h.hot);

    let block = make_block(1, vec![], 0);
    h.cold.append_block(block).await.unwrap();

    let result =
        rpc_call(&h.app, "getTransactionCount", json!([format!("{:?}", TEST_ADDR), "latest"]))
            .await;

    assert_eq!(result, json!("0x5"));
}

#[tokio::test]
async fn test_get_storage_at() {
    let h = TestHarness::new(1).await;
    setup_hot_account(&h.hot);

    let block = make_block(1, vec![], 0);
    h.cold.append_block(block).await.unwrap();

    let slot = format!("{:#066x}", 42u64);
    let result =
        rpc_call(&h.app, "getStorageAt", json!([format!("{:?}", TEST_ADDR), slot, "latest"])).await;

    // 999 = 0x3e7, padded to 32 bytes
    let expected = format!("{:#066x}", 999u64);
    assert_eq!(result, json!(expected));
}

#[tokio::test]
async fn test_get_code() {
    let h = TestHarness::new(1).await;
    setup_hot_account(&h.hot);

    let block = make_block(1, vec![], 0);
    h.cold.append_block(block).await.unwrap();

    let result = rpc_call(&h.app, "getCode", json!([format!("{:?}", TEST_ADDR), "latest"])).await;

    assert_eq!(result, json!("0x60006000f3"));
}

#[tokio::test]
async fn test_get_balance_unknown_account() {
    let h = TestHarness::new(1).await;

    let block = make_block(1, vec![], 0);
    h.cold.append_block(block).await.unwrap();

    let unknown = Address::repeat_byte(0xff);
    let result = rpc_call(&h.app, "getBalance", json!([format!("{:?}", unknown), "latest"])).await;

    assert_eq!(result, json!("0x0"));
}

// ---------------------------------------------------------------------------
// Group 5: Logs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_logs_by_block_hash() {
    let h = TestHarness::new(0).await;

    // Create block with transactions that have logs
    let (tx0, _) = make_signed_tx(0);
    let block = make_block(1, vec![tx0], 2); // 2 logs per receipt
    h.cold.append_block(block).await.unwrap();
    h.tags.set_latest(1);

    // Get the block hash
    let block_result = rpc_call(&h.app, "getBlockByNumber", json!(["0x1", false])).await;
    let block_hash = block_result["hash"].as_str().unwrap().to_string();

    let result = rpc_call(
        &h.app,
        "getLogs",
        json!([{
            "blockHash": block_hash,
            "address": format!("{:?}", LOG_ADDR),
        }]),
    )
    .await;

    let logs = result.as_array().unwrap();
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0]["address"], json!(format!("{:?}", LOG_ADDR)));
    assert_eq!(logs[0]["blockNumber"], json!("0x1"));
    assert_eq!(logs[0]["logIndex"], json!("0x0"));
    assert_eq!(logs[1]["logIndex"], json!("0x1"));
}

#[tokio::test]
async fn test_get_logs_by_range() {
    let h = TestHarness::new(0).await;

    let (tx0, _) = make_signed_tx(0);
    let block = make_block(1, vec![tx0], 1);
    h.cold.append_block(block).await.unwrap();
    h.tags.set_latest(1);

    let result = rpc_call(
        &h.app,
        "getLogs",
        json!([{
            "fromBlock": "0x1",
            "toBlock": "0x1",
            "topics": [format!("{:?}", LOG_TOPIC)],
        }]),
    )
    .await;

    let logs = result.as_array().unwrap();
    assert_eq!(logs.len(), 1);
    assert!(logs[0]["topics"].as_array().unwrap().contains(&json!(format!("{:?}", LOG_TOPIC))));
}

#[tokio::test]
async fn test_get_logs_empty() {
    let h = TestHarness::new(0).await;

    let (tx0, _) = make_signed_tx(0);
    let block = make_block(1, vec![tx0], 0); // no logs
    h.cold.append_block(block).await.unwrap();
    h.tags.set_latest(1);

    let result = rpc_call(
        &h.app,
        "getLogs",
        json!([{
            "fromBlock": "0x1",
            "toBlock": "0x1",
            "address": format!("{:?}", LOG_ADDR),
        }]),
    )
    .await;

    assert_eq!(result.as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Group 6: Edge cases & errors
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_not_supported() {
    let h = TestHarness::new(0).await;
    let resp = rpc_call_raw(&h.app, "syncing", json!([])).await;
    assert!(resp.get("error").is_some());
    let msg = resp["error"]["message"].as_str().unwrap();
    assert!(msg.contains("not supported"), "unexpected error: {msg}");
}

#[tokio::test]
async fn test_send_raw_tx_no_cache() {
    let h = TestHarness::new(0).await;
    let resp = rpc_call_raw(&h.app, "sendRawTransaction", json!(["0x00"])).await;
    assert!(resp.get("error").is_some());
}

// ---------------------------------------------------------------------------
// Group 7: Gas & Fee Queries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_gas_price() {
    let h = TestHarness::new(0).await;

    // Create a block with txs that have gas_price (2 gwei) > base_fee (1 gwei)
    let (tx0, _) = make_signed_tx_with_gas_price(0, 2_000_000_000);
    let block = make_block(1, vec![tx0], 0);
    h.cold.append_block(block).await.unwrap();
    h.tags.set_latest(1);

    let result = rpc_call(&h.app, "gasPrice", json!([])).await;

    // tip = gas_price - base_fee = 2e9 - 1e9 = 1e9
    // gasPrice = tip + base_fee = 1e9 + 1e9 = 2e9 = 0x77359400
    assert_eq!(result, json!("0x77359400"));
}

#[tokio::test]
async fn test_max_priority_fee_per_gas() {
    let h = TestHarness::new(0).await;

    let (tx0, _) = make_signed_tx_with_gas_price(0, 2_000_000_000);
    let block = make_block(1, vec![tx0], 0);
    h.cold.append_block(block).await.unwrap();
    h.tags.set_latest(1);

    let result = rpc_call(&h.app, "maxPriorityFeePerGas", json!([])).await;

    // tip only = gas_price - base_fee = 1e9 = 0x3b9aca00
    assert_eq!(result, json!("0x3b9aca00"));
}

#[tokio::test]
async fn test_gas_price_empty_blocks() {
    let h = TestHarness::new(0).await;

    let block = make_block(1, vec![], 0);
    h.cold.append_block(block).await.unwrap();
    h.tags.set_latest(1);

    let result = rpc_call(&h.app, "gasPrice", json!([])).await;

    // No txs means tip = 0, gasPrice = base_fee = 1e9 = 0x3b9aca00
    assert_eq!(result, json!("0x3b9aca00"));
}

#[tokio::test]
async fn test_fee_history_basic() {
    let h = TestHarness::new(0).await;

    for i in 1u64..=3 {
        let (tx, _) = make_signed_tx_with_gas_price(i - 1, 2_000_000_000);
        let block = make_block(i, vec![tx], 0);
        h.cold.append_block(block).await.unwrap();
    }
    h.tags.set_latest(3);

    // Request 2 blocks of fee history ending at block 3
    let result = rpc_call(&h.app, "feeHistory", json!(["0x2", "0x3", null])).await;

    // oldest_block = end_block + 1 - block_count = 3 + 1 - 2 = 2
    assert_eq!(result["oldestBlock"], json!("0x2"));
    // base_fee_per_gas has block_count + 1 entries (includes next-block prediction)
    let base_fees = result["baseFeePerGas"].as_array().unwrap();
    assert_eq!(base_fees.len(), 3);
    // gas_used_ratio has block_count entries
    let gas_ratios = result["gasUsedRatio"].as_array().unwrap();
    assert_eq!(gas_ratios.len(), 2);
    // No reward field when no percentiles requested
    assert!(result["reward"].is_null());
}

#[tokio::test]
async fn test_fee_history_with_rewards() {
    let h = TestHarness::new(0).await;

    let (tx0, _) = make_signed_tx_with_gas_price(0, 2_000_000_000);
    let block = make_block(1, vec![tx0], 0);
    h.cold.append_block(block).await.unwrap();
    h.tags.set_latest(1);

    let result = rpc_call(&h.app, "feeHistory", json!(["0x1", "0x1", [25.0, 75.0]])).await;

    assert_eq!(result["oldestBlock"], json!("0x1"));
    let rewards = result["reward"].as_array().unwrap();
    assert_eq!(rewards.len(), 1);
    let block_rewards = rewards[0].as_array().unwrap();
    assert_eq!(block_rewards.len(), 2);
}

// ---------------------------------------------------------------------------
// Group 8: Filters
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_new_block_filter_and_changes() {
    let h = TestHarness::new(0).await;

    // Install a block filter at block 0
    let filter_id = rpc_call(&h.app, "newBlockFilter", json!([])).await;
    let filter_id_str = filter_id.as_str().unwrap().to_string();

    // Append a block
    let (tx0, _) = make_signed_tx(0);
    let block = make_block(1, vec![tx0], 0);
    h.cold.append_block(block).await.unwrap();
    h.tags.set_latest(1);

    // Poll for changes — should get block hash for block 1
    let changes = rpc_call(&h.app, "getFilterChanges", json!([filter_id_str])).await;
    let hashes = changes.as_array().unwrap();
    assert_eq!(hashes.len(), 1);
    assert!(hashes[0].is_string());

    // Poll again with no new blocks — should be empty
    let changes = rpc_call(&h.app, "getFilterChanges", json!([filter_id_str])).await;
    let hashes = changes.as_array().unwrap();
    assert!(hashes.is_empty());
}

#[tokio::test]
async fn test_new_log_filter_and_changes() {
    let h = TestHarness::new(0).await;

    // Install a log filter for LOG_ADDR with LOG_TOPIC
    let filter_id = rpc_call(
        &h.app,
        "newFilter",
        json!([{
            "address": format!("{:?}", LOG_ADDR),
            "topics": [format!("{:?}", LOG_TOPIC)],
        }]),
    )
    .await;
    let filter_id_str = filter_id.as_str().unwrap().to_string();

    // Append a block with matching logs
    let (tx0, _) = make_signed_tx(0);
    let block = make_block(1, vec![tx0], 2);
    h.cold.append_block(block).await.unwrap();
    h.tags.set_latest(1);

    // Poll for changes — should get matching logs
    let changes = rpc_call(&h.app, "getFilterChanges", json!([filter_id_str])).await;
    let logs = changes.as_array().unwrap();
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0]["address"], json!(format!("{:?}", LOG_ADDR)));
}

#[tokio::test]
async fn test_uninstall_filter() {
    let h = TestHarness::new(0).await;

    let filter_id = rpc_call(&h.app, "newBlockFilter", json!([])).await;
    let filter_id_str = filter_id.as_str().unwrap().to_string();

    // Uninstall
    let result = rpc_call(&h.app, "uninstallFilter", json!([filter_id_str])).await;
    assert_eq!(result, json!(true));

    // Uninstall again — should return false
    let result = rpc_call(&h.app, "uninstallFilter", json!([filter_id_str])).await;
    assert_eq!(result, json!(false));
}

// ---------------------------------------------------------------------------
// Group 9: Debug namespace
// ---------------------------------------------------------------------------

/// Set up hot storage with a genesis header and fund an address.
///
/// The genesis header at block 0 is required so `revm_reader_at_height`
/// can validate height bounds. Without it, MemKv returns `NoBlocks`.
fn setup_hot_for_evm(hot: &MemKv, addr: Address, balance: U256) {
    use signet_storage_types::{Account, SealedHeader};

    let writer = hot.writer().unwrap();

    // Write a genesis header so the hot storage tracks block 0.
    let genesis = SealedHeader::new(Header::default());
    writer.put_header(&genesis).unwrap();

    writer.put_account(&addr, &Account { nonce: 0, balance, bytecode_hash: None }).unwrap();
    writer.commit().unwrap();
}

#[tokio::test]
async fn test_trace_block_by_number_noop() {
    let h = TestHarness::new(0).await;

    let (tx0, sender) = make_signed_tx(0);
    setup_hot_for_evm(&h.hot, sender, U256::from(1_000_000_000_000_000_000u128));

    let block = make_block(1, vec![tx0], 0);
    h.cold.append_block(block).await.unwrap();
    h.tags.set_latest(1);

    let debug_app = h.debug_app();
    let result =
        rpc_call(&debug_app, "traceBlockByNumber", json!(["0x1", {"tracer": "noopTracer"}])).await;

    let traces = result.as_array().unwrap();
    assert_eq!(traces.len(), 1);
}

#[tokio::test]
async fn test_trace_transaction_noop() {
    let h = TestHarness::new(0).await;

    let (tx0, sender) = make_signed_tx(0);
    let tx_hash = *tx0.tx_hash();
    setup_hot_for_evm(&h.hot, sender, U256::from(1_000_000_000_000_000_000u128));

    let block = make_block(1, vec![tx0], 0);
    h.cold.append_block(block).await.unwrap();
    h.tags.set_latest(1);

    let debug_app = h.debug_app();
    let result = rpc_call(
        &debug_app,
        "traceTransaction",
        json!([format!("{:?}", tx_hash), {"tracer": "noopTracer"}]),
    )
    .await;

    // NoopFrame result is not null
    assert!(!result.is_null());
}
