mod auth;
mod commands;
mod configuration;
#[cfg(feature = "desktop_harness_host")]
mod desktop_harness_host;
mod error;
#[cfg(feature = "desktop_harness_host")]
mod host_admission;
#[cfg(feature = "desktop_harness_host")]
mod host_capability;
#[cfg(feature = "desktop_harness_host")]
mod host_control_sink;
#[cfg(feature = "desktop_harness_host")]
mod host_document_analysis;
#[cfg(feature = "desktop_harness_host")]
mod host_event_sink;
mod host_helpers;
#[cfg(feature = "desktop_harness_host")]
mod host_persistence;
mod host_prompt;
mod host_provider;
#[cfg(feature = "desktop_harness_host")]
mod host_stream;
#[cfg(feature = "desktop_harness_host")]
mod host_task;
#[cfg(feature = "desktop_harness_host")]
mod host_task_runtime;
#[cfg(feature = "desktop_harness_host")]
mod host_tool_dispatch;
mod host_workspace;
#[cfg(feature = "desktop_harness_host")]
mod host_workspace_runtime;
#[cfg(feature = "desktop_harness_host")]
mod host_workspace_service;
#[cfg(feature = "desktop_harness_host")]
mod host_workspace_store;
#[cfg(feature = "desktop_harness_host")]
mod host_workspace_types;
mod logging;
mod openapi;
mod routes;
mod state;
#[cfg(feature = "team")]
mod team;

use agime::config::paths::Paths;
use agime_mcp::{
    mcp_server_runner::{serve, McpCommand},
    AutoVisualiserRouter, ComputerControllerServer, DeveloperServer, MemoryServer, TutorialServer,
};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the agent server
    Agent,
    /// Run the MCP server
    Mcp {
        #[arg(value_parser = clap::value_parser!(McpCommand))]
        server: McpCommand,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Agent => {
            commands::agent::run().await?;
        }
        Commands::Mcp { server } => {
            logging::setup_logging(Some(&format!("mcp-{}", server.name())))?;
            match server {
                McpCommand::AutoVisualiser => serve(AutoVisualiserRouter::new()).await?,
                McpCommand::ComputerController => serve(ComputerControllerServer::new()).await?,
                McpCommand::Memory => serve(MemoryServer::new()).await?,
                McpCommand::Tutorial => serve(TutorialServer::new()).await?,
                McpCommand::Developer => {
                    let bash_env = Paths::config_dir().join(".bash_env");
                    serve(
                        DeveloperServer::new()
                            .extend_path_with_shell(true)
                            .bash_env_file(Some(bash_env)),
                    )
                    .await?
                }
            }
        }
    }

    Ok(())
}
