//! Metrics to track
//!
//! - Counters:
//!   - Number of builder blocks extracted
//!   - Number of builder blocks processed
//!   - Number of transactions processed
//!   - Host blocks without builder blocks
//! - Histograms:
//!   - enter events extracted per block
//!   - enter token events extracted per block
//!   - transact events extracted per block
//!   - enter events processed per block
//!   - enter token events processed per block
//!   - transact events processed
//!   - Transaction counts per builder block

use alloy::consensus::BlockHeader;
use metrics::{Counter, Histogram, counter, describe_counter, describe_histogram, histogram};
use signet_evm::BlockResult;
use signet_extract::{Extractable, Extracts, HasTxns};
use signet_types::{MagicSig, MagicSigInfo};
use std::sync::LazyLock;

const BUILDER_BLOCKS_EXTRACTED: &str = "signet.block_processor.builder_blocks.extracted";
const BUILDER_BLOCKS_EXTRACTED_HELP: &str = "Number of builder blocks extracted from host";

const BLOCKS_PROCESSED: &str = "signet.block_processor.processed";
const BLOCKS_PROCESSED_HELP: &str = "Number of signet blocks processed";

const TRANSACTIONS_PROCESSED: &str = "signet.block_processor.transactions_processed";
const TRANSACTIONS_PROCESSED_HELP: &str = "Number of transactions processed in signet blocks";

const HOST_WITHOUT_BUILDER_BLOCK: &str = "signet.block_processor.host_without_builder_block";
const HOST_WITHOUT_BUILDER_BLOCK_HELP: &str = "Number of host blocks without builder blocks";

const TRANSACTIONS_PER_BUILDER_BLOCK: &str = "signet.block_processor.txns_per_builder_block";
const TRANSACTIONS_PER_BUILDER_BLOCK_HELP: &str =
    "Histogram of number of transactions per builder block";

const TRANSACT_EXTRACTS: &str = "signet.block_processor.transact_events.extracted";
const TRANSACT_EXTRACTS_HELP: &str = "Transact events extracted from host per block";

const ENTER_EXTRACTS: &str = "signet.block_processor.enter_events.extracted";
const ENTER_EXTRACTS_HELP: &str = "Enter events extracted from host per block";

const ENTER_TOKEN_EXTRACTS: &str = "signet.block_processor.enter_token_events.extracted";
const ENTER_TOKEN_EXTRACTS_HELP: &str =
    "Enter token events extracted from host per block, labeled by token";

const ENTER_PROCESSED: &str = "signet.block_processor.enter_events.processed";
const ENTER_PROCESSED_HELP: &str = "Histogram of number of enter events processed per block";

const ENTER_TOKEN_PROCESSED: &str = "signet.block_processor.enter_token_events.processed";
const ENTER_TOKEN_PROCESSED_HELP: &str =
    "Histogram of number of enter token events processed per block";

const TRANSACT_PROCESSED: &str = "signet.block_processor.transact_events.processed";
const TRANSACT_PROCESSED_HELP: &str = "Histogram of number of transact events processed per block";

const EXTRACTION_TIME: &str = "signet.block_processor.extraction.time";
const EXTRACTION_TIME_HELP: &str = "Time taken to extract signet outputs from a host notification. Note: sometimes the extraction includes multiple blocks.";

const PROCESSING_TIME: &str = "signet.block_processor.processing.time";
const PROCESSING_TIME_HELP: &str =
    "Time taken to process a single signet block from extracts, in milliseconds.";

const BLOCK_GAS_USED: &str = "signet.block_processor.block.gas_used";
const BLOCK_GAS_USED_HELP: &str = "Gas used per signet block processed.";

static DESCRIBE: LazyLock<()> = LazyLock::new(|| {
    describe_counter!(BUILDER_BLOCKS_EXTRACTED, BUILDER_BLOCKS_EXTRACTED_HELP);
    describe_counter!(BLOCKS_PROCESSED, BLOCKS_PROCESSED_HELP);
    describe_counter!(TRANSACTIONS_PROCESSED, TRANSACTIONS_PROCESSED_HELP);
    describe_counter!(HOST_WITHOUT_BUILDER_BLOCK, HOST_WITHOUT_BUILDER_BLOCK_HELP);

    describe_histogram!(TRANSACTIONS_PER_BUILDER_BLOCK, TRANSACTIONS_PER_BUILDER_BLOCK_HELP);
    describe_histogram!(TRANSACT_EXTRACTS, TRANSACT_EXTRACTS_HELP);
    describe_histogram!(ENTER_EXTRACTS, ENTER_EXTRACTS_HELP);
    describe_histogram!(ENTER_TOKEN_EXTRACTS, ENTER_TOKEN_EXTRACTS_HELP);
    describe_histogram!(ENTER_PROCESSED, ENTER_PROCESSED_HELP);
    describe_histogram!(ENTER_TOKEN_PROCESSED, ENTER_TOKEN_PROCESSED_HELP);
    describe_histogram!(TRANSACT_PROCESSED, TRANSACT_PROCESSED_HELP);
    describe_histogram!(EXTRACTION_TIME, EXTRACTION_TIME_HELP);
    describe_histogram!(PROCESSING_TIME, PROCESSING_TIME_HELP);
    describe_histogram!(BLOCK_GAS_USED, BLOCK_GAS_USED_HELP);
});

fn blocks_extracted() -> Counter {
    LazyLock::force(&DESCRIBE);
    counter!(BUILDER_BLOCKS_EXTRACTED)
}

fn inc_blocks_extracted() {
    blocks_extracted().increment(1);
}

fn blocks_processed() -> Counter {
    LazyLock::force(&DESCRIBE);
    counter!(BLOCKS_PROCESSED)
}

fn inc_blocks_processed() {
    blocks_processed().increment(1);
}

fn transactions_processed() -> Counter {
    LazyLock::force(&DESCRIBE);
    counter!(TRANSACTIONS_PROCESSED)
}

fn inc_transactions_processed(value: u64) {
    transactions_processed().increment(value);
}

fn host_without_builder_block() -> Counter {
    LazyLock::force(&DESCRIBE);
    counter!(HOST_WITHOUT_BUILDER_BLOCK)
}

fn inc_host_without_builder_block() {
    host_without_builder_block().increment(1);
}

fn transactions_per_builder_block() -> Histogram {
    LazyLock::force(&DESCRIBE);
    histogram!(TRANSACTIONS_PER_BUILDER_BLOCK)
}

fn record_transactions_per_builder_block(value: u64) {
    transactions_per_builder_block().record(value as f64);
}

fn transact_extracts() -> Histogram {
    LazyLock::force(&DESCRIBE);
    histogram!(TRANSACT_EXTRACTS)
}

fn record_transact_extracts(value: u64) {
    transact_extracts().record(value as f64);
}

fn enter_extracts() -> Histogram {
    LazyLock::force(&DESCRIBE);
    histogram!(ENTER_EXTRACTS)
}

fn record_enter_extracts(value: u64) {
    enter_extracts().record(value as f64);
}

fn enter_token_extracts() -> Histogram {
    LazyLock::force(&DESCRIBE);
    histogram!(ENTER_TOKEN_EXTRACTS)
}

fn record_enter_token_events(value: u64) {
    enter_token_extracts().record(value as f64);
}

fn enters_processed() -> Histogram {
    LazyLock::force(&DESCRIBE);
    histogram!(ENTER_PROCESSED)
}

fn record_enters_processed(value: u64) {
    enters_processed().record(value as f64);
}

fn enter_token_processed() -> Histogram {
    LazyLock::force(&DESCRIBE);
    histogram!(ENTER_TOKEN_PROCESSED)
}

fn record_enter_token_processed(value: u64) {
    enter_token_processed().record(value as f64);
}

fn transacts_processed() -> Histogram {
    LazyLock::force(&DESCRIBE);
    histogram!(TRANSACT_PROCESSED)
}

fn record_transacts_processed(value: u64) {
    transacts_processed().record(value as f64);
}

fn extraction_time() -> Histogram {
    LazyLock::force(&DESCRIBE);
    histogram!(EXTRACTION_TIME)
}

pub(crate) fn record_extraction_time(started_at: &std::time::Instant) {
    extraction_time().record(started_at.elapsed().as_millis() as f64);
}

fn processing_time() -> Histogram {
    LazyLock::force(&DESCRIBE);
    histogram!(PROCESSING_TIME)
}

fn record_processing_time(started_at: &std::time::Instant) {
    processing_time().record(started_at.elapsed().as_millis() as f64);
}

fn block_gas_used() -> Histogram {
    LazyLock::force(&DESCRIBE);
    histogram!(BLOCK_GAS_USED)
}

fn record_block_gas_used(value: u64) {
    block_gas_used().record(value as f64);
}

pub(crate) fn record_extracts<T: Extractable>(extracts: &Extracts<'_, T>) {
    record_enter_extracts(extracts.enters.len() as u64);
    record_enter_token_events(extracts.enter_tokens.len() as u64);
    record_transact_extracts(extracts.transacts.len() as u64);
    if extracts.events.submitted.is_some() {
        inc_blocks_extracted();
    } else {
        inc_host_without_builder_block();
    }
}

pub(crate) fn record_block_result(block: &BlockResult, started_at: &std::time::Instant) {
    inc_blocks_processed();
    inc_transactions_processed(block.sealed_block.transactions().len() as u64);
    record_processing_time(started_at);

    // find the index of the first magic sig transaction
    // That index is the count of builder block transactions
    let txns = block.sealed_block.transactions();

    let txns_processed =
        txns.partition_point(|tx| MagicSig::try_from_signature(tx.signature()).is_none());

    let sys_txns = &txns[txns_processed..];

    let mut enters = 0;
    let mut enter_tokens = 0;
    let mut transacts = 0;
    for tx in sys_txns.iter() {
        match MagicSig::try_from_signature(tx.signature()) {
            Some(MagicSig { ty: MagicSigInfo::Enter, .. }) => {
                enters += 1;
            }
            Some(MagicSig { ty: MagicSigInfo::EnterToken, .. }) => {
                enter_tokens += 1;
            }
            Some(MagicSig { ty: MagicSigInfo::Transact { .. }, .. }) => {
                transacts += 1;
            }
            Some(_) | None => unreachable!(),
        };
    }

    record_block_gas_used(block.sealed_block().gas_used());
    record_transactions_per_builder_block(txns_processed as u64);
    record_enters_processed(enters);
    record_enter_token_processed(enter_tokens);
    record_transacts_processed(transacts);
}
