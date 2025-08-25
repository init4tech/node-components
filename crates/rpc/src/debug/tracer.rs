use crate::DebugError;
use reth::rpc::{
    server_types::eth::EthApiError,
    types::trace::geth::{
        FourByteFrame, GethDebugBuiltInTracerType, GethDebugTracerConfig, GethDebugTracerType,
        GethDebugTracingOptions, GethTrace,
    },
};
use revm_inspectors::tracing::{FourByteInspector, TracingInspector, TracingInspectorConfig};
use signet_evm::EvmReady;
use trevm::{
    helpers::Ctx,
    revm::{Database, DatabaseCommit, Inspector},
};

pub(super) fn trace<Db: Database, Insp: Inspector<Ctx<Db>>>(
    trevm: EvmReady<Db, Insp>,
    config: &GethDebugTracingOptions,
) -> Result<GethTrace, DebugError> {
    let Some(tracer) = &config.tracer else { todo!() };

    let GethDebugTracerType::BuiltInTracer(built_in) = tracer else {
        return Err(EthApiError::Unsupported("JS tracer").into());
    };

    match built_in {
        GethDebugBuiltInTracerType::FourByteTracer => trace_four_byte(trevm).map_err(Into::into),
        GethDebugBuiltInTracerType::CallTracer => {
            trace_call(&config.tracer_config, trevm).map_err(Into::into)
        }
        GethDebugBuiltInTracerType::FlatCallTracer => todo!(),
        GethDebugBuiltInTracerType::PreStateTracer => todo!(),
        GethDebugBuiltInTracerType::NoopTracer => todo!(),
        GethDebugBuiltInTracerType::MuxTracer => todo!(),
    }
}

fn trace_four_byte<Db: Database + DatabaseCommit, Insp: Inspector<Ctx<Db>>>(
    trevm: EvmReady<Db, Insp>,
) -> Result<GethTrace, EthApiError> {
    let mut four_byte = FourByteInspector::default();

    let trevm = trevm.try_with_inspector(&mut four_byte, |trevm| trevm.run());

    let trevm = trevm.map_err(|e| EthApiError::EvmCustom(e.into_error().to_string()))?;

    trevm.accept_state();

    Ok(FourByteFrame::from(four_byte).into())
}

fn trace_call<Db: Database, Insp: Inspector<Ctx<Db>>>(
    tracer_config: &GethDebugTracerConfig,
    trevm: EvmReady<Db, Insp>,
) -> Result<GethTrace, EthApiError> {
    let call_config =
        tracer_config.clone().into_call_config().map_err(|_| EthApiError::InvalidTracerConfig)?;
    let mut inspector =
        TracingInspector::new(TracingInspectorConfig::from_geth_call_config(&call_config));

    let trevm = trevm.try_with_inspector(&mut inspector, |trevm| trevm.run());

    let trevm = trevm.map_err(|e| EthApiError::EvmCustom(e.into_error().to_string()))?;

    inspector.set_transaction_gas_limit(trevm.gas_limit());
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
