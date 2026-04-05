use clap::{Parser, Subcommand};
use tracing::error;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "sinew",
    version,
    about = "Peer discovery and messaging for Claude Code sessions"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the Broker daemon
    Broker {
        /// Port to listen on
        #[arg(short, long, default_value_t = 7899)]
        port: u16,
    },
    /// Start the MCP server (stdio transport)
    Serve {
        /// Broker address
        #[arg(short, long, default_value = "http://127.0.0.1:7899")]
        broker_url: String,
    },
    /// Shutdown the running Broker daemon
    Shutdown {
        /// Broker address
        #[arg(short, long, default_value = "http://127.0.0.1:7899")]
        broker_url: String,
    },
    /// Show status of the Broker and connected peers
    Status {
        /// Broker address
        #[arg(short, long, default_value = "http://127.0.0.1:7899")]
        broker_url: String,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Broker { port } => {
            if let Err(report) = sinew::broker::run_broker(port).await {
                error!("Broker failed:\n{report:?}");
                std::process::exit(1);
            }
        }
        Command::Serve { broker_url } => {
            if let Err(report) = sinew::mcp::lifecycle::run_mcp_server(&broker_url).await {
                error!("MCP server failed:\n{report:?}");
                std::process::exit(1);
            }
        }
        Command::Shutdown { broker_url } => {
            let client = sinew::mcp::client::BrokerClient::new(&broker_url);
            match client.shutdown().await {
                Ok(()) => println!("Broker shutdown requested successfully."),
                Err(e) => {
                    eprintln!("Failed to shutdown broker: {e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Status { broker_url } => {
            let client = sinew::mcp::client::BrokerClient::new(&broker_url);
            match client.health().await {
                Ok(health) => {
                    println!("Broker: {} ({})", health.status, broker_url);
                    println!("Peers:  {}", health.peer_count);
                }
                Err(e) => {
                    eprintln!("Broker not reachable: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}
