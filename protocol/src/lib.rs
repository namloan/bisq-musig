mod protocol_musig_adaptor;
mod nigiri;

#[cfg(test)]
mod tests {
    use crate::nigiri;
    use crate::protocol_musig_adaptor::{BMPContext, BMPProtocol, ProtocolRole};
    use bdk_electrum::bdk_core::bitcoin::Amount;

    #[test]
    fn test_musig() -> anyhow::Result<()> {
        println!("running...");
        nigiri::check_start();
        let mut alice_funds = nigiri::funded_wallet();
        //TestWallet::new()?;

        let bob_funds = nigiri::funded_wallet();
        //TestWallet::new()?;
        nigiri::fund_wallet(&mut alice_funds);
        let seller_amount = &Amount::from_btc(1.4)?;
        let buyer_amount = &Amount::from_btc(0.2)?;

        // init p3 --------------------------
        let alice_context = BMPContext::new(alice_funds, ProtocolRole::Seller, seller_amount.clone(), buyer_amount.clone())?;

        let mut alice = BMPProtocol::new(alice_context)?;
        let bob_context = BMPContext::new(bob_funds, ProtocolRole::Buyer, seller_amount.clone(), buyer_amount.clone())?;
        let mut bob = BMPProtocol::new(bob_context)?;
        nigiri::tiktok();

        // Round 1--------
        let alice_response = alice.round1()?;
        let bob_response = bob.round1()?;

        // Round2 -------
        let alice_r2 = alice.round2(bob_response)?;
        let bob_r2 = bob.round2(alice_response)?;

        println!("P2TR P' {}", alice.p_tik.get_agg_adr()?.to_string());
        println!("P2TR Q' {}", alice.q_tik.get_agg_adr()?.to_string());

        assert!(alice.get_p_tik_agg() == bob.get_p_tik_agg());
        assert!(alice.q_tik.agg_point == bob.q_tik.agg_point);

        // Round 3 ----------
        let alice_r3 = alice.round3(bob_r2)?;
        let bob_r3 = bob.round3(alice_r2)?;

        assert_eq!(alice_r3.deposit_txid, bob_r3.deposit_txid);
        dbg!(&alice_r3.deposit_txid);

        // Round 4 ---------------------------
        let alice_r4 = alice.round4(bob_r3)?;
        let bob_r4 = bob.round4(alice_r3)?;

        // Round 5 --------------------------
        alice.round5(bob_r4)?;
        bob.round5(alice_r4)?;


        // done -----------------------------
        crate::nigiri::tiktok();
        Ok(())
    }
}
