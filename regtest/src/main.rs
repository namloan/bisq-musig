fn main() {
    // common::setup();

    let alice_wallet = "alice";
    let bob_wallet = "bob";
    let delay = time::Duration::from_secs(2); // Add delays between steps

    println!("Creating wallet for Alice...");
    Command::new("nigiri")
        .args(["rpc", "createwallet", alice_wallet])
        .output()
        .expect("Failed to create Alice's wallet");

    println!("Creating wallet for Bob...");
    Command::new("nigiri")
        .args(["rpc", "createwallet", bob_wallet])
        .output()
        .expect("Failed to create Bob's wallet");

    println!("Getting a new address for Alice...");
    let alice_address = Command::new("nigiri")
        .args(["rpc", "-rpcwallet=alice", "getnewaddress"])
        .output()
        .expect("Failed to get Alice's address");
    let alice_address = String::from_utf8_lossy(&alice_address.stdout)
        .trim()
        .to_string();
    println!("Alice's address: {}", alice_address);
    assert!(!alice_address.is_empty(), "Alice's address is empty!");

    println!("Getting a new address for Bob...");
    let bob_address = Command::new("nigiri")
        .args(["rpc", "-rpcwallet=bob", "getnewaddress"])
        .output()
        .expect("Failed to get Bob's address");
    let bob_address = String::from_utf8_lossy(&bob_address.stdout)
        .trim()
        .to_string();
    println!("Bob's address: {}", bob_address);
    assert!(!bob_address.is_empty(), "Bob's address is empty!");

    println!("Funding Alice's wallet with the faucet...");
    let faucet_response = Command::new("nigiri")
        .args(["faucet", &alice_address])
        .output()
        .expect("Failed to fund Alice's wallet");
    println!("{}", String::from_utf8_lossy(&faucet_response.stdout));
    thread::sleep(delay);

    // Confirm Alice's balance after funding
    println!("Confirming Alice's balance after faucet funding...");
    let alice_balance = Command::new("nigiri")
        .args(["rpc", "-rpcwallet=alice", "getbalance"])
        .output()
        .expect("Failed to get Alice's balance");
    let alice_balance: f64 = String::from_utf8_lossy(&alice_balance.stdout)
        .trim()
        .parse()
        .expect("Failed to parse Alice's balance");
    println!("Alice's balance after funding: {}", alice_balance);
    assert_eq!(
        alice_balance, 1.0,
        "Alice's balance should be exactly 1 BTC after faucet funding!"
    );

    // Ensure faucet funding is confirmed by mining a block
    println!("Mining a block to confirm faucet funding...");
    Command::new("nigiri")
        .args(["rpc", "generatetoaddress", "1", &alice_address])
        .output()
        .expect("Failed to mine block");
    thread::sleep(delay);

    println!("Sending funds from Alice to Bob...");
    let send_response = Command::new("nigiri")
        .args([
            "rpc",
            "-rpcwallet=alice",
            "sendtoaddress",
            &bob_address,
            "0.01", // Amount to send in BTC
        ])
        .output()
        .expect("Failed to send funds from Alice to Bob");
    let transaction_id = String::from_utf8_lossy(&send_response.stdout)
        .trim()
        .to_string();
    assert!(
        !transaction_id.is_empty(),
        "Transaction ID is empty! Transaction failed."
    );
    println!("Transaction ID: {}", transaction_id);

    println!("Mining a new block to confirm the transaction...");
    Command::new("nigiri")
        .args(["rpc", "generatetoaddress", "1", &alice_address])
        .output()
        .expect("Failed to mine a new block");
    thread::sleep(delay);

    println!("Forcing Bob's wallet to rescan the blockchain...");
    Command::new("nigiri")
        .args(["rpc", "-rpcwallet=bob", "rescanblockchain"])
        .output()
        .expect("Failed to rescan Bob's wallet");
    thread::sleep(delay);

    println!("Verifying Alice's balance...");
    let alice_balance_after = Command::new("nigiri")
        .args(["rpc", "-rpcwallet=alice", "getbalance"])
        .output()
        .expect("Failed to get Alice's balance");
    let alice_balance_after: f64 = String::from_utf8_lossy(&alice_balance_after.stdout)
        .trim()
        .parse()
        .expect("Failed to parse Alice's balance after transaction");
    println!(
        "Alice's balance after sending funds: {}",
        alice_balance_after
    );
    assert!(
        approx_eq(alice_balance_after, 0.99),
        "Alice's balance should be approximately 0.99 BTC after sending 0.01 BTC! Found: {}",
        alice_balance_after
    );

    println!("Verifying Bob's balance...");
    let bob_balance = Command::new("nigiri")
        .args(["rpc", "-rpcwallet=bob", "getbalance"])
        .output()
        .expect("Failed to get Bob's balance");
    let bob_balance: f64 = String::from_utf8_lossy(&bob_balance.stdout)
        .trim()
        .parse()
        .expect("Failed to parse Bob's balance");
    println!("Bob's balance after receiving funds: {}", bob_balance);
    assert!(
        approx_eq(bob_balance, 0.01),
        "Bob's balance should be approximately 0.01 BTC after receiving funds! Found: {}",
        bob_balance
    );

    println!("Test passed: Alice's and Bob's balances are as expected.");
}
mod bisq_musig_integration_test;
mod common;

use std::{process::Command, thread, time};

const EPSILON: f64 = 0.00001; // Allowable margin of error for balance checks

fn approx_eq(left: f64, right: f64) -> bool {
    (left - right).abs() < EPSILON
}
