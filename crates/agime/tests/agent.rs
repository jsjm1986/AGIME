use std::sync::Arc;

use agime::agents::{Agent, AgentEvent};
use agime::config::extensions::{set_extension, ExtensionEntry};
use anyhow::Result;
use futures::StreamExt;

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod retry_tests {
        use super::*;
        use agime::agents::types::{RetryConfig, SuccessCheck};

        #[tokio::test]
        async fn test_retry_success_check_execution() -> Result<()> {
            use agime::agents::retry::execute_success_checks;

            let retry_config = RetryConfig {
                max_retries: 3,
                checks: vec![],
                on_failure: None,
                timeout_seconds: Some(30),
                on_failure_timeout_seconds: Some(60),
            };

            let success_checks = vec![SuccessCheck::Shell {
                command: "echo 'test'".to_string(),
            }];

            let result = execute_success_checks(&success_checks, &retry_config).await;
            assert!(result.is_ok(), "Success check should pass");
            assert!(result.unwrap(), "Command should succeed");

            let fail_checks = vec![SuccessCheck::Shell {
                command: "false".to_string(),
            }];

            let result = execute_success_checks(&fail_checks, &retry_config).await;
            assert!(result.is_ok(), "Success check execution should not error");
            assert!(!result.unwrap(), "Command should fail");

            Ok(())
        }

        #[tokio::test]
        async fn test_retry_logic_with_validation_errors() -> Result<()> {
            let invalid_retry_config = RetryConfig {
                max_retries: 0,
                checks: vec![],
                on_failure: None,
                timeout_seconds: Some(0),
                on_failure_timeout_seconds: None,
            };

            let validation_result = invalid_retry_config.validate();
            assert!(
                validation_result.is_err(),
                "Should validate max_retries > 0"
            );
            assert!(validation_result
                .unwrap_err()
                .contains("max_retries must be greater than 0"));

            Ok(())
        }

        #[tokio::test]
        async fn test_retry_attempts_counter_reset() -> Result<()> {
            let agent = Agent::new();

            agent.reset_retry_attempts().await;
            let initial_attempts = agent.get_retry_attempts().await;
            assert_eq!(initial_attempts, 0);

            let new_attempts = agent.increment_retry_attempts().await;
            assert_eq!(new_attempts, 1);

            agent.reset_retry_attempts().await;
            let reset_attempts = agent.get_retry_attempts().await;
            assert_eq!(reset_attempts, 0);

            Ok(())
        }
    }

    #[cfg(test)]
    mod max_turns_tests {
        use super::*;
        use agime::agents::SessionConfig;
        use agime::conversation::message::{Message, MessageContent};
        use agime::model::ModelConfig;
        use agime::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
        use agime::providers::errors::ProviderError;
        use agime::session::session_manager::SessionType;
        use agime::session::SessionManager;
        use async_trait::async_trait;
        use rmcp::model::{CallToolRequestParams, Tool};
        use rmcp::object;
        use std::path::PathBuf;

        struct MockToolProvider {}

        impl MockToolProvider {
            fn new() -> Self {
                Self {}
            }
        }

        #[async_trait]
        impl Provider for MockToolProvider {
            async fn complete(
                &self,
                _system_prompt: &str,
                _messages: &[Message],
                _tools: &[Tool],
            ) -> Result<(Message, ProviderUsage), ProviderError> {
                let tool_call = CallToolRequestParams {
                    name: "test_tool".into(),
                    arguments: Some(object!({"param": "value"})),
                    meta: None,
                    task: None,
                };
                let message = Message::assistant().with_tool_request("call_123", Ok(tool_call));

                let usage = ProviderUsage::new(
                    "mock-model".to_string(),
                    Usage::new(Some(10), Some(5), Some(15)),
                );

                Ok((message, usage))
            }

            async fn complete_with_model(
                &self,
                _model_config: &ModelConfig,
                system_prompt: &str,
                messages: &[Message],
                tools: &[Tool],
            ) -> anyhow::Result<(Message, ProviderUsage), ProviderError> {
                self.complete(system_prompt, messages, tools).await
            }

            fn get_model_config(&self) -> ModelConfig {
                ModelConfig::new("mock-model").unwrap()
            }

            fn metadata() -> ProviderMetadata {
                ProviderMetadata {
                    name: "mock".to_string(),
                    display_name: "Mock Provider".to_string(),
                    description: "Mock provider for testing".to_string(),
                    default_model: "mock-model".to_string(),
                    known_models: vec![],
                    model_doc_link: "".to_string(),
                    config_keys: vec![],
                }
            }

            fn get_name(&self) -> &str {
                "mock-test"
            }
        }

        #[tokio::test]
        async fn test_max_turns_limit() -> Result<()> {
            let agent = Agent::new();
            let provider = Arc::new(MockToolProvider::new());
            let user_message = Message::user().with_text("Hello");

            let session = SessionManager::create_session(
                PathBuf::default(),
                "max-turn-test".to_string(),
                SessionType::Hidden,
            )
            .await?;

            agent.update_provider(provider, &session.id).await?;

            let session_config = SessionConfig {
                id: session.id,
                schedule_id: None,
                max_turns: None,
                retry_config: None,
            };

            let reply_stream = agent.reply(user_message, session_config, None).await?;
            tokio::pin!(reply_stream);

            let mut responses = Vec::new();
            while let Some(response_result) = reply_stream.next().await {
                match response_result {
                    Ok(AgentEvent::Message(response)) => {
                        if let Some(MessageContent::ToolConfirmationRequest(ref req)) =
                            response.content.first()
                        {
                            agent.handle_confirmation(
                            req.id.clone(),
                            agime::permission::PermissionConfirmation {
                                principal_type: agime::permission::permission_confirmation::PrincipalType::Tool,
                                permission: agime::permission::Permission::AllowOnce,
                            }
                        ).await;
                        }
                        responses.push(response);
                    }
                    Ok(AgentEvent::McpNotification(_)) => {}
                    Ok(AgentEvent::ModelChange { .. }) => {}
                    Ok(AgentEvent::ToolTransportRequest(_)) => {}
                    Ok(AgentEvent::HistoryReplaced(_updated_conversation)) => {
                        // We should update the conversation here, but we're not reading it
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }

            assert!(
                !responses.is_empty(),
                "Expected at least 1 response, got {}",
                responses.len()
            );

            // Look for the max turns message as the last response
            let last_response = responses.last().unwrap();
            let last_content = last_response.content.first().unwrap();
            if let MessageContent::Text(text_content) = last_content {
                assert!(text_content.text.contains(
                    "I've reached the maximum number of actions I can do without user input"
                ));
            } else {
                panic!("Expected text content in last message");
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod extension_manager_tests {
        use super::*;
        use agime::agents::extension::{ExtensionConfig, PlatformExtensionContext};
        use agime::agents::extension_manager_extension::{
            MANAGE_EXTENSIONS_TOOL_NAME, SEARCH_AVAILABLE_EXTENSIONS_TOOL_NAME,
        };

        async fn setup_agent_with_extension_manager() -> Agent {
            // Add the Tasks extension to the config so it can be discovered by search_available_extensions
            // Set it as disabled initially so tests can enable it
            let todo_extension_entry = ExtensionEntry {
                enabled: false,
                config: ExtensionConfig::Platform {
                    name: "tasks".to_string(),
                    description:
                        "Enable structured task tracking for Goose so it can keep track of what it is doing"
                            .to_string(),
                    bundled: Some(true),
                    available_tools: vec![],
                },
            };
            set_extension(todo_extension_entry);

            let agent = Agent::new();

            agent
                .extension_manager
                .set_context(PlatformExtensionContext {
                    session_id: Some("test_session".to_string()),
                    task_board_context: None,
                    extension_manager: Some(Arc::downgrade(&agent.extension_manager)),
                    tool_route_manager: Some(Arc::downgrade(&agent.tool_route_manager)),
                })
                .await;

            // Now add the extension manager platform extension
            let ext_config = ExtensionConfig::Platform {
                name: "extensionmanager".to_string(),
                description: "Extension Manager".to_string(),
                bundled: Some(true),
                available_tools: vec![],
            };

            agent
                .add_extension(ext_config)
                .await
                .expect("Failed to add extension manager");
            agent
        }

        #[tokio::test]
        async fn test_extension_manager_tools_available() {
            let agent = setup_agent_with_extension_manager().await;
            let tools = agent.list_tools(None).await;

            // Note: Tool names are prefixed with the normalized extension name "extensionmanager"
            // not the display name "Extension Manager"
            let search_tool = tools.iter().find(|tool| {
                tool.name == format!("extensionmanager__{SEARCH_AVAILABLE_EXTENSIONS_TOOL_NAME}")
            });
            assert!(
                search_tool.is_some(),
                "search_available_extensions tool should be available"
            );

            let manage_tool = tools.iter().find(|tool| {
                tool.name == format!("extensionmanager__{MANAGE_EXTENSIONS_TOOL_NAME}")
            });
            assert!(
                manage_tool.is_some(),
                "manage_extensions tool should be available"
            );
        }
    }
}
