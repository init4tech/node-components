//! This file is largely adapted from reth: `crates/rpc/rpc/src/debug.rs`
//!
//! In particular the `debug_trace_call` function.

use crate::DebugError;
use reth::{
    revm::{DatabaseRef, context::ContextTr},
    rpc::{
        server_types::eth::EthApiError,
        types::{
            TransactionInfo,
            trace::geth::{
                FourByteFrame, GethDebugBuiltInTracerType, GethDebugTracerConfig,
                GethDebugTracerType, GethDebugTracingOptions, GethTrace, NoopFrame,
            },
        },
    },
};
use revm_inspectors::tracing::{
    FourByteInspector, MuxInspector, TracingInspector, TracingInspectorConfig,
};
use signet_evm::{EvmNeedsTx, EvmReady};
use tracing::instrument;
use trevm::{
    helpers::Ctx,
    revm::{Database, DatabaseCommit, Inspector},
};

/// Trace a transaction using the provided EVM and tracing options.
#[instrument(skip(trevm, config, tx_info), fields(tx_hash = ?tx_info.hash))]
pub(super) fn trace<Db, Insp>(
    trevm: EvmReady<Db, Insp>,
    config: &GethDebugTracingOptions,
    tx_info: TransactionInfo,
) -> Result<(GethTrace, EvmNeedsTx<Db, Insp>), DebugError>
where
    Db: Database + DatabaseCommit + DatabaseRef,
    Insp: Inspector<Ctx<Db>>,
{
    let Some(tracer) = &config.tracer else { return Err(EthApiError::InvalidTracerConfig.into()) };

    let GethDebugTracerType::BuiltInTracer(built_in) = tracer else {
        return Err(EthApiError::Unsupported("JS tracer").into());
    };

    match built_in {
        GethDebugBuiltInTracerType::FourByteTracer => trace_four_byte(trevm).map_err(Into::into),
        GethDebugBuiltInTracerType::CallTracer => {
            trace_call(&config.tracer_config, trevm).map_err(Into::into)
        }
        GethDebugBuiltInTracerType::FlatCallTracer => {
            trace_flat_call(&config.tracer_config, trevm, tx_info).map_err(Into::into)
        }
        GethDebugBuiltInTracerType::PreStateTracer => {
            trace_pre_state(&config.tracer_config, trevm).map_err(Into::into)
        }
        GethDebugBuiltInTracerType::NoopTracer => Ok((
            NoopFrame::default().into(),
            trevm
                .run()
                .map_err(|err| EthApiError::EvmCustom(err.into_error().to_string()))?
                .accept_state(),
        )),
        GethDebugBuiltInTracerType::MuxTracer => {
            trace_mux(&config.tracer_config, trevm, tx_info).map_err(Into::into)
        }
    }
}

/// Traces a call using [`GethDebugBuiltInTracerType::FourByteTracer`].
fn trace_four_byte<Db, Insp>(
    trevm: EvmReady<Db, Insp>,
) -> Result<(GethTrace, EvmNeedsTx<Db, Insp>), EthApiError>
where
    Db: Database + DatabaseCommit,
    Insp: Inspector<Ctx<Db>>,
{
    let mut four_byte = FourByteInspector::default();

    let trevm = trevm.try_with_inspector(&mut four_byte, |trevm| trevm.run());

    let trevm = trevm.map_err(|e| EthApiError::EvmCustom(e.into_error().to_string()))?;

    Ok((FourByteFrame::from(four_byte).into(), trevm.accept_state()))
}

/// Traces a call using [`GethDebugBuiltInTracerType::CallTracer`].
fn trace_call<Db, Insp>(
    tracer_config: &GethDebugTracerConfig,
    trevm: EvmReady<Db, Insp>,
) -> Result<(GethTrace, EvmNeedsTx<Db, Insp>), EthApiError>
where
    Db: Database + DatabaseCommit,
    Insp: Inspector<Ctx<Db>>,
{
    let call_config =
        tracer_config.clone().into_call_config().map_err(|_| EthApiError::InvalidTracerConfig)?;

    let mut inspector =
        TracingInspector::new(TracingInspectorConfig::from_geth_call_config(&call_config));

    let trevm = trevm.try_with_inspector(&mut inspector, |trevm| trevm.run());

    let trevm = trevm.map_err(|e| EthApiError::EvmCustom(e.into_error().to_string()))?;

    let frame = inspector
        .with_transaction_gas_limit(trevm.gas_limit())
        .into_geth_builder()
        .geth_call_traces(call_config, trevm.gas_used());

    Ok((frame.into(), trevm.accept_state()))
}

/// Traces a call using [`GethDebugBuiltInTracerType::PreStateTracer`]
fn trace_pre_state<Db, Insp>(
    tracer_config: &GethDebugTracerConfig,
    trevm: EvmReady<Db, Insp>,
) -> Result<(GethTrace, EvmNeedsTx<Db, Insp>), EthApiError>
where
    Db: Database + DatabaseCommit + DatabaseRef,
    Insp: Inspector<Ctx<Db>>,
{
    let prestate_config = tracer_config
        .clone()
        .into_pre_state_config()
        .map_err(|_| EthApiError::InvalidTracerConfig)?;

    let mut inspector =
        TracingInspector::new(TracingInspectorConfig::from_geth_prestate_config(&prestate_config));

    let trevm = trevm.try_with_inspector(&mut inspector, |trevm| trevm.run());

    let trevm = trevm.map_err(|e| EthApiError::EvmCustom(e.into_error().to_string()))?;
    let gas_limit = trevm.gas_limit();

    // NB: Normally we would call `trevm.accept_state()` here, but we need the
    // state after execution to be UNCOMMITED when we compute the prestate
    // diffs.
    let (result, mut trevm) = trevm.take_result_and_state();

    let frame = inspector
        .with_transaction_gas_limit(gas_limit)
        .into_geth_builder()
        .geth_prestate_traces(&result, &prestate_config, trevm.inner_mut_unchecked().db_mut())
        .map_err(|err| EthApiError::EvmCustom(err.to_string()))?;

    // This is equivalent to calling `trevm.accept_state()`.
    trevm.inner_mut_unchecked().db_mut().commit(result.state);

    Ok((frame.into(), trevm))
}

fn trace_flat_call<Db, Insp>(
    tracer_config: &GethDebugTracerConfig,
    trevm: EvmReady<Db, Insp>,
    tx_info: TransactionInfo,
) -> Result<(GethTrace, EvmNeedsTx<Db, Insp>), EthApiError>
where
    Db: Database + DatabaseCommit,
    Insp: Inspector<Ctx<Db>>,
{
    let flat_call_config = tracer_config
        .clone()
        .into_flat_call_config()
        .map_err(|_| EthApiError::InvalidTracerConfig)?;

    let mut inspector =
        TracingInspector::new(TracingInspectorConfig::from_flat_call_config(&flat_call_config));

    let trevm = trevm.try_with_inspector(&mut inspector, |trevm| trevm.run());

    let trevm = trevm.map_err(|e| EthApiError::EvmCustom(e.into_error().to_string()))?;

    let frame = inspector
        .with_transaction_gas_limit(trevm.gas_limit())
        .into_parity_builder()
        .into_localized_transaction_traces(tx_info);

    Ok((frame.into(), trevm.accept_state()))
}

fn trace_mux<Db, Insp>(
    tracer_config: &GethDebugTracerConfig,
    trevm: EvmReady<Db, Insp>,
    tx_info: TransactionInfo,
) -> Result<(GethTrace, EvmNeedsTx<Db, Insp>), EthApiError>
where
    Db: Database + DatabaseCommit + DatabaseRef,
    Insp: Inspector<Ctx<Db>>,
{
    let mux_config =
        tracer_config.clone().into_mux_config().map_err(|_| EthApiError::InvalidTracerConfig)?;

    let mut inspector = MuxInspector::try_from_config(mux_config)
        .map_err(|err| EthApiError::EvmCustom(err.to_string()))?;

    let trevm = trevm.try_with_inspector(&mut inspector, |trevm| trevm.run());
    let trevm = trevm.map_err(|e| EthApiError::EvmCustom(e.into_error().to_string()))?;

    // NB: Normally we would call `trevm.accept_state()` here, but we need the
    // state after execution to be UNCOMMITED when we compute the prestate
    // diffs.
    let (result, mut trevm) = trevm.take_result_and_state();

    let frame = inspector
        .try_into_mux_frame(&result, trevm.inner_mut_unchecked().db_mut(), tx_info)
        .map_err(|err| EthApiError::EvmCustom(err.to_string()))?;

    // This is equivalent to calling `trevm.accept_state()`.
    trevm.inner_mut_unchecked().db_mut().commit(result.state);

    Ok((frame.into(), trevm))
}

// Some code in this file has been copied and modified from reth
// <https://github.com/paradigmxyz/reth>
// The original license is included below:
//
// The MIT License (MIT)
//
// Copyright (c) 2022-2025 Reth Contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//.
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
