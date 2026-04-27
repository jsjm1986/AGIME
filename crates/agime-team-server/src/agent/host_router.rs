use anyhow::Result;
use serde_json::Value;

use super::server_harness_host::{current_host_execution_path, HostExecutionPath};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatHostRouteRequest {
    pub session_id: String,
    pub agent_id: String,
    pub user_message: String,
    pub workspace_path: String,
    pub turn_system_instruction: Option<String>,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    pub validation_mode: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelHostRouteRequest {
    pub channel_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub user_message: String,
    pub workspace_path: String,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    pub validation_mode: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocumentAnalysisHostRouteRequest {
    pub session_id: String,
    pub agent_id: String,
    pub user_message: String,
    pub workspace_path: String,
    pub llm_overrides: Option<Value>,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    pub validation_mode: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct HostExecutionRouter {
    path: HostExecutionPath,
}

impl HostExecutionRouter {
    pub fn from_env() -> Self {
        Self {
            path: current_host_execution_path(),
        }
    }

    #[cfg(test)]
    pub fn new(path: HostExecutionPath) -> Self {
        Self { path }
    }

    pub fn path(&self) -> HostExecutionPath {
        self.path
    }

    pub async fn route_chat<T, FD, FutD>(
        &self,
        request: ChatHostRouteRequest,
        direct: FD,
    ) -> Result<T>
    where
        FD: FnOnce(ChatHostRouteRequest) -> FutD,
        FutD: std::future::Future<Output = Result<T>>,
    {
        match self.path {
            HostExecutionPath::DirectHarness => direct(request).await,
        }
    }

    pub async fn route_channel<T, FD, FutD>(
        &self,
        request: ChannelHostRouteRequest,
        direct: FD,
    ) -> Result<T>
    where
        FD: FnOnce(ChannelHostRouteRequest) -> FutD,
        FutD: std::future::Future<Output = Result<T>>,
    {
        match self.path {
            HostExecutionPath::DirectHarness => direct(request).await,
        }
    }

    pub async fn route_document_analysis<T, FD, FutD>(
        &self,
        request: DocumentAnalysisHostRouteRequest,
        direct: FD,
    ) -> Result<T>
    where
        FD: FnOnce(DocumentAnalysisHostRouteRequest) -> FutD,
        FutD: std::future::Future<Output = Result<T>>,
    {
        match self.path {
            HostExecutionPath::DirectHarness => direct(request).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use super::*;

    #[tokio::test]
    async fn channel_router_uses_direct_path() {
        let router = HostExecutionRouter::new(HostExecutionPath::DirectHarness);
        let called = Arc::new(Mutex::new(Vec::new()));
        let request = ChannelHostRouteRequest {
            channel_id: "channel-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            user_message: "hello".to_string(),
            workspace_path: "/tmp/workspace".to_string(),
            target_artifacts: Vec::new(),
            result_contract: Vec::new(),
            validation_mode: false,
        };

        let result = router
            .route_channel(
                request,
                {
                    let called = called.clone();
                    |req| async move {
                        called
                            .lock()
                            .await
                            .push(format!("direct:{}", req.channel_id));
                        Ok("direct")
                    }
                },
            )
            .await
            .expect("route should succeed");

        assert_eq!(result, "direct");
        assert_eq!(called.lock().await.as_slice(), ["direct:channel-1"]);
    }

    #[tokio::test]
    async fn document_router_preserves_llm_overrides_for_direct_path() {
        let router = HostExecutionRouter::new(HostExecutionPath::DirectHarness);
        let request = DocumentAnalysisHostRouteRequest {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            user_message: "analyze".to_string(),
            workspace_path: "/tmp/workspace".to_string(),
            llm_overrides: Some(serde_json::json!({ "model": "claude-test" })),
            target_artifacts: vec!["document:doc-1".to_string()],
            result_contract: vec!["document:doc-1".to_string()],
            validation_mode: true,
        };

        let seen_model = router
            .route_document_analysis(
                request,
                |req| async move {
                    Ok(req
                        .llm_overrides
                        .and_then(|value| value.get("model").cloned())
                        .and_then(|value| value.as_str().map(str::to_string)))
                },
            )
            .await
            .expect("route should succeed");

        assert_eq!(seen_model.as_deref(), Some("claude-test"));
    }
}
