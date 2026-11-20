// SPDX-License-Identifier: GPL-3.0-or-later
//! `nectarlink-agent`: what AI agents run to use NectarLink, shipped with the
//! app. `mcp` serves MCP over stdio, `do` runs one action, `actions` lists
//! them, `events` follows what happens. Everything goes through the running app (Settings > AI agents).

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "nectarlink-agent",
    version,
    about = "Lets AI agents use NectarLink, through the running app"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Serve NectarLink over MCP on stdin and stdout.
    Mcp {
        /// The agent's name, as added in NectarLink.
        #[arg(long, env = "NECTARLINK_AGENT")]
        agent: String,
        /// Only three tools (search, describe, run), for clients with a tool limit.
        #[arg(long)]
        dispatch: bool,
        /// Only these toolsets (comma separated), e.g. core,messages.
        #[arg(long, value_delimiter = ',')]
        toolsets: Vec<String>,
    },
    /// Run one action and print its JSON answer.
    Do {
        /// The action, e.g. devices.list or sms.send.
        action: String,
        #[arg(long, env = "NECTARLINK_AGENT")]
        agent: String,
        /// The arguments as a JSON object.
        #[arg(long)]
        args: Option<String>,
        /// The device, by id or name.
        #[arg(long)]
        device: Option<String>,
        /// Indented output.
        #[arg(long)]
        pretty: bool,
    },
    /// Print events as they happen, one JSON object per line, until stopped.
    Events {
        #[arg(long, env = "NECTARLINK_AGENT")]
        agent: String,
        /// Only these kinds (comma separated), e.g. notification.posted,sms.changed.
        #[arg(long, value_delimiter = ',')]
        kinds: Vec<String>,
        /// Start after this event number instead of now.
        #[arg(long)]
        since: Option<u64>,
        /// Print what's there and stop instead of following.
        #[arg(long)]
        once: bool,
    },
    /// List the actions this agent may use.
    Actions {
        #[arg(long, env = "NECTARLINK_AGENT")]
        agent: String,
        #[arg(long)]
        pretty: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let code = runtime.block_on(async {
        match &cli.command {
            Command::Mcp { agent, dispatch, toolsets } => {
                nectarlink_mcp::cli::mcp(agent, *dispatch, toolsets).await
            }
            Command::Do { action, agent, args, device, pretty } => {
                nectarlink_mcp::cli::run(agent, action, args.as_deref(), device.as_deref(), *pretty).await
            }
            Command::Actions { agent, pretty } => nectarlink_mcp::cli::list(agent, *pretty).await,
            Command::Events { agent, kinds, since, once } => {
                nectarlink_mcp::cli::events(agent, kinds, *since, *once).await
            }
        }
    })?;
    std::process::exit(code);
}
