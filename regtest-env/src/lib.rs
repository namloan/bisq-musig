//! Common test utilities for Bitcoin-related testing
//! 
//! This crate provides common utilities for setting up test environments,
//! particularly for Bitcoin-related testing using Nigiri.

use std::process::Command;
use std::thread;
use std::time::Duration;

/// Sets up a test environment with Nigiri (Bitcoin test environment)
/// 
/// This function:
/// 1. Checks if Docker is running
/// 2. Stops any existing Nigiri instance
/// 3. Cleans up existing Bitcoin container and data
/// 4. Starts a fresh Nigiri instance
/// 5. Waits for services to be ready
/// 
/// # Panics
/// 
/// This function will panic if:
/// - Docker is not running or not installed
/// - Nigiri fails to start
pub fn setup() {
    println!("Starting the setup...");

    // Check if Docker is running
    let docker_check = Command::new("docker")
        .arg("info")
        .output();

    match docker_check {
        Ok(output) => {
            if !output.status.success() {
                eprintln!("Docker is not running. Please start Docker first.");
                std::process::exit(1);
            }
        }
        Err(_) => {
            eprintln!("Docker is not installed or not in PATH. Please install Docker first.");
            std::process::exit(1);
        }
    }

    // Stop Nigiri first
    println!("Stopping Nigiri...");
    let _ = Command::new("nigiri")
        .arg("stop")
        .output();

    // Force remove any existing Bitcoin container
    println!("Removing existing Bitcoin container...");
    let _ = Command::new("docker")
        .args(["rm", "-f", "nigiri-bitcoin"])
        .output();

    // Remove Bitcoin data volume
    println!("Removing Bitcoin data volume...");
    let _ = Command::new("docker")
        .args(["volume", "rm", "-f", "nigiri-bitcoin-data"])
        .output();

    // Create a temporary container to clean up wallet data
    println!("Cleaning up wallet data...");
    let _ = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            "nigiri-bitcoin-data:/data",
            "alpine",
            "sh",
            "-c",
            "rm -rf /data/.bitcoin/regtest/wallets/*"
        ])
        .output();

    // Start fresh Nigiri instance
    println!("Starting fresh Nigiri instance...");
    let nigiri_output = Command::new("nigiri")
        .arg("start")
        .output();

    match nigiri_output {
        Ok(output) => {
            if output.status.success() {
                println!("Nigiri started successfully.");
            } else {
                eprintln!(
                    "Failed to start Nigiri. Error: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error while starting Nigiri: {}", e);
            std::process::exit(1);
        }
    }

    // Wait for Nigiri's Bitcoin node to be fully ready
    println!("Waiting for Nigiri services to be ready...");
    thread::sleep(Duration::from_secs(2));

    println!("Setup completed successfully.");
}

/// Funds a Bitcoin address using Nigiri's faucet and mines a block
/// 
/// # Arguments
/// 
/// * `address` - The Bitcoin address to fund
pub fn fund_address(address: &str) {
    let faucet_response = Command::new("nigiri")
        .args(["faucet", address])
        .output()
        .expect("Failed to fund wallet");
    eprintln!("{}", String::from_utf8_lossy(&faucet_response.stdout));

    eprintln!("Mining to {}", address);
    let resp = mine(address, 1);
    eprintln!("response {}", String::from_utf8_lossy(&resp.stdout));
}

/// Mines a specified number of blocks to a given address using Nigiri
/// 
/// # Arguments
/// 
/// * `address` - The Bitcoin address to mine to
/// * `num_blocks` - Number of blocks to mine
pub fn mine(address: &str, num_blocks: u16) -> std::process::Output {
    Command::new("nigiri")
        .args(["rpc", "generatetoaddress", &num_blocks.to_string(), address])
        .output()
        .expect("Failed to mine block")
} 