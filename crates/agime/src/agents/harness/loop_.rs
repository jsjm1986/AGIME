use super::{
    context::HarnessContext, policy::HarnessPolicy, state::HarnessState,
    transcript::SessionHarnessStore,
};

#[derive(Debug, Clone)]
pub struct HarnessRunLoop {
    pub context: HarnessContext,
    pub state: HarnessState,
    pub policy: HarnessPolicy,
    pub transcript_store: SessionHarnessStore,
}

impl HarnessRunLoop {
    pub fn new(
        context: HarnessContext,
        state: HarnessState,
        policy: HarnessPolicy,
        transcript_store: SessionHarnessStore,
    ) -> Self {
        Self {
            context,
            state,
            policy,
            transcript_store,
        }
    }

    pub async fn run<F, Fut, T>(self, f: F) -> T
    where
        F: FnOnce(HarnessContext, HarnessState, HarnessPolicy, SessionHarnessStore) -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        f(self.context, self.state, self.policy, self.transcript_store).await
    }
}
