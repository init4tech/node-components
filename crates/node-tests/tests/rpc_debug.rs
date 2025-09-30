use alloy::{primitives::Bytes, providers::ext::DebugApi, sol_types::SolCall};
use reth::{
    providers::TransactionsProvider,
    rpc::types::trace::geth::{CallConfig, GethDebugTracingOptions},
};
use serial_test::serial;
use signet_node_tests::{aliases::Counter::incrementCall, rpc::rpc_test};
use signet_test_utils::specs::{HostBlockSpec, RuBlockSpec};

#[serial]
#[tokio::test]
async fn test_debug_trace_transaction() {
    rpc_test(|ctx, counter| async move {
        let deployer = ctx.addresses[0];
        let deploy_tx = &ctx.factory.transactions_by_block(1.into()).unwrap().unwrap()[0];
        let tx_hash = *deploy_tx.hash();

        let tracing_opts = GethDebugTracingOptions::call_tracer(CallConfig {
            only_top_call: Some(false),
            with_log: Some(true),
        });

        let trace = ctx
            .alloy_provider
            .debug_trace_transaction(tx_hash, tracing_opts)
            .await
            .expect("debug_traceTransaction failed")
            .try_into_call_frame()
            .unwrap();

        assert_eq!(trace.typ, "CREATE");
        assert_eq!(trace.from, deployer);
        assert_eq!(trace.to, Some(*counter.address()));

        ctx
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_debug_trace_block() {
    rpc_test(|ctx, counter| async move {
        let mut spec = RuBlockSpec::new(ctx.constants());

        // We want 10 transactions in this block
        for i in 0..10 {
            let tx = counter.increment().into_transaction_request().from(ctx.addresses[i]);

            spec = spec.alloy_tx(&ctx.fill_alloy_tx(&tx).await.unwrap());
        }

        ctx.process_block(HostBlockSpec::new(ctx.constants()).submit_block(spec)).await.unwrap();

        let tracing_opts = GethDebugTracingOptions::call_tracer(CallConfig {
            only_top_call: Some(false),
            with_log: Some(true),
        });

        let traces = ctx
            .alloy_provider
            .debug_trace_block_by_number(2.into(), tracing_opts)
            .await
            .expect("debug_traceBlock failed");

        assert_eq!(traces.len(), 10);

        for trace in traces {
            let trace = trace.success().unwrap().clone().try_into_call_frame().unwrap();
            assert_eq!(trace.typ, "CALL");
            assert_eq!(trace.to, Some(*counter.address()));
            assert_eq!(trace.input, Bytes::from_static(&incrementCall::SELECTOR));
        }

        ctx
    })
    .await;
}
