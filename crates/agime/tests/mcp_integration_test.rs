use serde::Deserialize;

use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::{env, fs};

use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::object;
use tokio_util::sync::CancellationToken;

use agime::agents::extension::{Envs, ExtensionConfig};
use agime::agents::extension_manager::ExtensionManager;
use agime::model::ModelConfig;

use test_case::test_case;

use agime::conversation::message::Message;
use agime::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use agime::providers::errors::ProviderError;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::process::Command;

#[derive(Deserialize)]
struct CargoBuildMessage {
    reason: String,
    target: Target,
    executable: String,
}

#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
}

#[derive(Clone)]
pub struct MockProvider {
    pub model_config: ModelConfig,
}

impl MockProvider {
    pub fn new(model_config: ModelConfig) -> Self {
        Self { model_config }
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "mock"
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> anyhow::Result<(Message, ProviderUsage), ProviderError> {
        Ok((
            Message::assistant().with_text("\"So we beat on, boats against the current, borne back ceaselessly into the past.\" — F. Scott Fitzgerald, The Great Gatsby (1925)"),
            ProviderUsage::new("mock".to_string(), Usage::default()),
        ))
    }

    fn get_model_config(&self) -> ModelConfig {
        self.model_config.clone()
    }
}

fn build_and_get_binary_path() -> PathBuf {
    let output = Command::new("cargo")
        .args([
            "build",
            "--frozen",
            "-p",
            "agime-test",
            "--bin",
            "capture",
            "--message-format=json",
        ])
        .output()
        .expect("failed to build binary");

    if !output.status.success() {
        panic!("build failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(serde_json::from_str::<CargoBuildMessage>)
        .filter_map(Result::ok)
        .filter(|message| message.reason == "compiler-artifact")
        .filter_map(|message| {
            if message.target.name == "capture"
                && message.target.kind.contains(&String::from("bin"))
            {
                Some(PathBuf::from(message.executable))
            } else {
                None
            }
        })
        .next()
        .expect("failed to parse binary path")
}

static REPLAY_BINARY_PATH: Lazy<PathBuf> = Lazy::new(build_and_get_binary_path);

enum TestMode {
    Record,
    Playback,
}

#[test_case(
    vec!["npx", "-y", "@modelcontextprotocol/server-everything"],
    vec![
        CallToolRequestParams { name: "echo".into(), arguments: Some(object!({"message": "Hello, world!" })), meta: None, task: None },
        CallToolRequestParams { name: "add".into(), arguments: Some(object!({"a": 1, "b": 2 })), meta: None, task: None },
        CallToolRequestParams { name: "longRunningOperation".into(), arguments: Some(object!({"duration": 1, "steps": 5 })), meta: None, task: None },
        CallToolRequestParams { name: "structuredContent".into(), arguments: Some(object!({"location": "11238"})), meta: None, task: None },
        CallToolRequestParams { name: "sampleLLM".into(), arguments: Some(object!({"prompt": "Please provide a quote from The Great Gatsby", "maxTokens": 100 })), meta: None, task: None }
    ],
    vec![]
)]
#[test_case(
    vec!["github-mcp-server", "stdio"],
    vec![
        CallToolRequestParams { name: "get_file_contents".into(), arguments: Some(object!({
            "owner": "block",
            "repo": "goose",
            "path": "README.md",
            "sha": "ab62b863c1666232a67048b6c4e10007a2a5b83c"
        })), meta: None, task: None},
    ],
    vec!["GITHUB_PERSONAL_ACCESS_TOKEN"]
)]
#[test_case(
    vec!["uvx", "mcp-server-fetch"],
    vec![
        CallToolRequestParams { name: "fetch".into(), arguments: Some(object!({
            "url": "https://example.com",
        })), meta: None, task: None }
    ],
    vec![]
)]
#[test_case(
    vec!["cargo", "run", "--quiet", "-p", "agime-server", "--bin", "agimed", "--", "mcp", "developer"],
    vec![
        CallToolRequestParams { name: "text_editor".into(), arguments: Some(object!({
            "command": "view",
            "path": "/tmp/goose_test/goose.txt"
        })), meta: None, task: None},
        CallToolRequestParams { name: "text_editor".into(), arguments: Some(object!({
            "command": "str_replace",
            "path": "/tmp/goose_test/goose.txt",
            "old_str": "# goose",
            "new_str": "# goose (modified by test)"
        })), meta: None, task: None},
        // Test shell command to verify file was modified
        CallToolRequestParams { name: "shell".into(), arguments: Some(object!({
            "command": "cat /tmp/goose_test/goose.txt"
        })), meta: None, task: None },
        // Test text_editor tool to restore original content
        CallToolRequestParams { name: "text_editor".into(), arguments: Some(object!({
            "command": "str_replace",
            "path": "/tmp/goose_test/goose.txt",
            "old_str": "# goose (modified by test)",
            "new_str": "# goose"
        })), meta: None, task: None},
        CallToolRequestParams { name: "list_windows".into(), arguments: Some(object!({})), meta: None, task: None },
    ],
    vec![]
)]
#[tokio::test]
async fn test_replayed_session(
    command: Vec<&str>,
    tool_calls: Vec<CallToolRequestParams>,
    required_envs: Vec<&str>,
) {
    std::env::set_var("GOOSE_MCP_CLIENT_VERSION", "0.0.0");

    // Setup test file for developer extension tests
    let test_file_path = "/tmp/goose_test/goose.txt";
    if let Some(parent) = std::path::Path::new(test_file_path).parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(test_file_path, "# goose\n").ok();
    let replay_name_from_command = |parts: &[&str]| -> String {
        parts
            .iter()
            .map(|s| s.replace("/", "_"))
            .collect::<Vec<String>>()
            .join("")
    };
    let mut replay_file_name = replay_name_from_command(&command);
    let mut replay_file_path =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("should find the project root"));
    replay_file_path.push("tests");
    replay_file_path.push("mcp_replays");
    replay_file_path.push(&replay_file_name);

    // Compatibility shim for older replay fixtures created before goose->agime rename.
    // Playback prefers current naming, then falls back to legacy fixture name if present.
    if env::var("GOOSE_RECORD_MCP").is_err() && !replay_file_path.exists() {
        let legacy_command = command
            .iter()
            .map(|part| {
                part.replace("agime-server", "goose-server")
                    .replace("agimed", "goosed")
                    .replace("agime", "goose")
            })
            .collect::<Vec<String>>();
        let legacy_command_refs = legacy_command
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>();
        let legacy_replay_name = replay_name_from_command(&legacy_command_refs);

        let mut legacy_path =
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("should find the project root"));
        legacy_path.push("tests");
        legacy_path.push("mcp_replays");
        legacy_path.push(&legacy_replay_name);

        if legacy_path.exists() {
            replay_file_name = legacy_replay_name;
            replay_file_path = legacy_path;
        }
    }

    let mode = if env::var("GOOSE_RECORD_MCP").is_ok() {
        TestMode::Record
    } else {
        assert!(replay_file_path.exists(), "replay file doesn't exist");
        TestMode::Playback
    };

    let mode_arg = match mode {
        TestMode::Record => "record",
        TestMode::Playback => "playback",
    };
    let cmd = REPLAY_BINARY_PATH.to_string_lossy().to_string();
    let mut args = vec!["stdio", mode_arg]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<String>>();

    args.push(replay_file_path.to_string_lossy().to_string());

    let mut env = HashMap::new();

    if matches!(mode, TestMode::Record) {
        args.extend(command.into_iter().map(str::to_string));

        for key in required_envs {
            match env::var(key) {
                Ok(v) => {
                    env.insert(key.to_string(), v);
                }
                Err(_) => {
                    eprintln!("skipping due to missing required env variable: {}", key);
                    return;
                }
            }
        }
    }

    let envs = Envs::new(env);
    let extension_config = ExtensionConfig::Stdio {
        name: "test".to_string(),
        description: "Test".to_string(),
        cmd,
        args,
        envs,
        env_keys: vec![],
        timeout: Some(30),
        bundled: Some(false),
        available_tools: vec![],
    };

    let provider = Arc::new(tokio::sync::Mutex::new(Some(Arc::new(MockProvider {
        model_config: ModelConfig::new("test-model").unwrap(),
    }) as Arc<dyn Provider>)));
    let extension_manager = ExtensionManager::new(provider);

    #[allow(clippy::redundant_closure_call)]
    let result = (async || -> Result<(), Box<dyn std::error::Error>> {
        extension_manager.add_extension(extension_config).await?;
        let mut results = Vec::new();
        for tool_call in tool_calls {
            let tool_call = CallToolRequestParams {
                name: format!("test__{}", tool_call.name).into(),
                arguments: tool_call.arguments,
                meta: None,
                task: None,
            };
            let result = extension_manager
                .dispatch_tool_call(tool_call, CancellationToken::default())
                .await;

            let tool_result = result?;
            results.push(tool_result.result.await?);
        }

        let mut results_path = replay_file_path.clone();
        results_path.pop();
        results_path.push(format!("{}.results.json", &replay_file_name));

        match mode {
            TestMode::Record => {
                serde_json::to_writer_pretty(File::create(results_path)?, &results)?
            }
            TestMode::Playback => assert_eq!(
                serde_json::from_reader::<_, Vec<CallToolResult>>(File::open(results_path)?)?,
                results
            ),
        };

        Ok(())
    })()
    .await;

    if let Err(err) = result {
        if matches!(mode, TestMode::Playback) {
            let errors =
                fs::read_to_string(format!("{}.errors.txt", replay_file_path.to_string_lossy()))
                    .expect("could not read errors");
            eprintln!("errors from {}", replay_file_path.to_string_lossy());
            eprintln!("{}", errors);
            eprintln!();
        }
        panic!("Test failed: {:?}", err);
    }
}
