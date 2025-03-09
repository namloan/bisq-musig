mod walletrpc;

use clap::{Parser, Subcommand};
use std::prelude::rust_2021::*;
use tonic::Request;

use crate::walletrpc::{ListUnspentRequest, NewAddressRequest, WalletBalanceRequest};
use crate::walletrpc::wallet_client::WalletClient;

#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Compute and display the wallet's current balance
    WalletBalance,
    /// Generate a new address
    NewAddress,
    /// List utxos available for spending
    ListUnspent,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli: Cli = Cli::parse();

    let mut client = WalletClient::connect("http://127.0.0.1:50051").await?;

    match cli.commands {
        Commands::WalletBalance => {
            let response = client.wallet_balance(Request::new(WalletBalanceRequest {})).await?;
            drop(client);
            println!("{:#?}", response);
        }
        Commands::NewAddress => {
            let response = client.new_address(Request::new(NewAddressRequest {})).await?;
            drop(client);
            println!("{:#?}", response);
        }
        Commands::ListUnspent => {
            let response = client.list_unspent(Request::new(ListUnspentRequest {})).await?;
            drop(client);
            println!("{:#?}", response);
        }
    }
    Ok(())
}
