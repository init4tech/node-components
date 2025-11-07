use crate::{context::SignetTestContext, types::TestCounterInstance, utils::run_test};

/// A test helper that sets up a Signet test context, deploys the Counter
/// contract, and then runs the provided async function `f` with the context and
/// the deployed contract instance.
pub async fn rpc_test<F, Fut>(f: F)
where
    F: FnOnce(SignetTestContext, TestCounterInstance) -> Fut + Send,
    Fut: Future<Output = SignetTestContext> + Send,
{
    run_test(|ctx| async move {
        let deployer = ctx.addresses[0];

        let instance = ctx.deploy_counter(deployer).await;

        f(ctx, instance).await;
    })
    .await;
}
