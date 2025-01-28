use crate::musig_adaptor_protocol::BMPContext;
use crate::{ProtocolRole, TestWallet};
use bdk_bitcoind_rpc::bitcoincore_rpc::bitcoin::Amount;

#[test]
fn test_musig() -> anyhow::Result<()> {
    println!("running...");
    crate::nigiri::check_start();
    let mut alice_funds = TestWallet::new()?;
    // crate::nigiri::funded_wallet();
    let bob_funds = TestWallet::new()?; //crate::nigiri::funded_wallet();
    // crate::nigiri::fund_wallet(&mut alice_funds);
    let seller_amount = &Amount::from_btc(1.4)?;
    let buyer_amount = &Amount::from_btc(0.2)?;

    let mut alice = BMPContext::new(alice_funds, ProtocolRole::Seller, seller_amount, buyer_amount)?;
    let mut bob = BMPContext::new(bob_funds, ProtocolRole::Buyer, seller_amount, buyer_amount)?;

    let alice_response = alice.round1()?;
    let bob_response = bob.round1()?;

    let alice_r2 = alice.round2(bob_response)?;
    let bob_r2 = bob.round2(alice_response)?;

    println!("{}", alice.get_p_tik_agg().to_string());
    // println!("P2TR Addres {}", alice.get_p_tik_agg());

    // assert!(alice.get_p_tik_agg() == bob.get_p_tik_agg());

    Ok(())
}