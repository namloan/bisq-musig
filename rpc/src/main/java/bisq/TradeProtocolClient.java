package bisq;

import helloworld.Helloworld;
import helloworld.MuSigGrpc;
import io.grpc.Grpc;
import io.grpc.InsecureChannelCredentials;

public class TradeProtocolClient {
    public static void main(String[] args) {
        var channel = Grpc.newChannelBuilderForAddress(
                "127.0.0.1",
                50051,
                InsecureChannelCredentials.create()
        ).build();

        var musigStub = MuSigGrpc.newBlockingStub(channel);
        testMusigService_twoParties(musigStub);

        channel.shutdown();
    }

    private static void testMusigService_twoParties(MuSigGrpc.MuSigBlockingStub stub) {
        // Two peers, buyer-as-taker & seller-as-maker, talk to their respective Rust servers via
        // gRPC, simulated here as two sessions (trade IDs) with the same test server.
        //
        // Communication with the gRPC server is interspersed with messages exchanged between the
        // peers. These are the messages A-G defined in $SRC_ROOT/musig_trade_protocol_messages.txt,
        // with messages A-D used to set up the trade. The Java client is (for the most part) just
        // forwarding on fields that were received in the last one or two gRPC responses.

        var buyerPubKeyShareResponse = stub.initTrade(Helloworld.PubKeySharesRequest.newBuilder()
                .setTradeId("buyer-trade")
                .setMyRole(Helloworld.Role.BUYER_AS_TAKER)
                .build());
        System.out.println("Got reply: " + buyerPubKeyShareResponse);

        // Buyer sends Message A to seller.

        var sellerPubKeyShareResponse = stub.initTrade(Helloworld.PubKeySharesRequest.newBuilder()
                .setTradeId("seller-trade")
                .setMyRole(Helloworld.Role.SELLER_AS_MAKER)
                .build());
        System.out.println("Got reply: " + sellerPubKeyShareResponse);

        var sellerNonceShareMessage = stub.getNonceShares(Helloworld.NonceSharesRequest.newBuilder()
                .setTradeId("seller-trade")
                .setBuyerOutputPeersPubKeyShare(buyerPubKeyShareResponse.getBuyerOutputPubKeyShare())
                .setSellerOutputPeersPubKeyShare(buyerPubKeyShareResponse.getSellerOutputPubKeyShare())
                .setDepositTxFeeRate(12.5)
                .setPreparedTxFeeRate(10.0)
                .setTradeAmount(200000)
                .setBuyersSecurityDeposit(30000)
                .setSellersSecurityDeposit(30000)
                .build());
        System.out.println("Got reply: " + sellerNonceShareMessage);

        // Seller sends Message B to buyer.

        var buyerNonceShareMessage = stub.getNonceShares(Helloworld.NonceSharesRequest.newBuilder()
                .setTradeId("buyer-trade")
                .setBuyerOutputPeersPubKeyShare(sellerPubKeyShareResponse.getBuyerOutputPubKeyShare())
                .setSellerOutputPeersPubKeyShare(sellerPubKeyShareResponse.getSellerOutputPubKeyShare())
                .setDepositTxFeeRate(12.5)
                .setPreparedTxFeeRate(10.0)
                .setTradeAmount(200000)
                .setBuyersSecurityDeposit(30000)
                .setSellersSecurityDeposit(30000)
                .build());
        System.out.println("Got reply: " + buyerNonceShareMessage);

        var buyerPartialSignatureMessage = stub.getPartialSignatures(Helloworld.PartialSignaturesRequest.newBuilder()
                .setTradeId("buyer-trade")
                .setPeersNonceShares(sellerNonceShareMessage)
                .build());
        System.out.println("Got reply: " + buyerPartialSignatureMessage);

        // Buyer sends Message C to seller.

        var sellerPartialSignatureMessage = stub.getPartialSignatures(Helloworld.PartialSignaturesRequest.newBuilder()
                .setTradeId("seller-trade")
                .setPeersNonceShares(buyerNonceShareMessage)
                .build());
        System.out.println("Got reply: " + sellerPartialSignatureMessage);

        var sellerDepositPsbt = stub.signDepositTx(Helloworld.DepositTxSignatureRequest.newBuilder()
                .setTradeId("seller-trade")
                .setPeersPartialSignatures(buyerPartialSignatureMessage.toBuilder().clearSwapTxInputPartialSignature()) // REDACT buyer's swapTxInputPartialSignature.
                .build());
        System.out.println("Got reply: " + sellerDepositPsbt);

        // Seller sends Message D to buyer.

        var buyerDepositPsbt = stub.signDepositTx(Helloworld.DepositTxSignatureRequest.newBuilder()
                .setTradeId("buyer-trade")
                .setPeersPartialSignatures(sellerPartialSignatureMessage)
                .build());
        System.out.println("Got reply: " + buyerDepositPsbt);

        // *** BUYER BROADCASTS DEPOSIT TX ***
        var depositTxConfirmationIter = stub.publishDepositTx(Helloworld.PublishDepositTxRequest.newBuilder()
                .setTradeId("buyer-trade")
                .build());
        depositTxConfirmationIter.forEachRemaining(reply -> System.out.println("Got reply: " + reply));
        // ***********************************

        // Buyer sends Message E to seller.

        var swapTxSignatureResponse = stub.signSwapTx(Helloworld.SwapTxSignatureRequest.newBuilder()
                .setTradeId("seller-trade")
                .setSwapTxInputPeersPartialSignature(buyerPartialSignatureMessage.getSwapTxInputPartialSignature()) // NOW send the redacted buyer's swapTxInputPartialSignature.
                .build());
        System.out.println("Got reply: " + swapTxSignatureResponse);

        // Seller sends Message F to buyer.

        // *** BUYER CLOSES TRADE ***
        var buyersCloseTradeResponse = stub.closeTrade(Helloworld.CloseTradeRequest.newBuilder()
                .setTradeId("buyer-trade")
                .setMyOutputPeersPrvKeyShare(swapTxSignatureResponse.getPeerOutputPrvKeyShare())
                .build());
        System.out.println("Got reply: " + buyersCloseTradeResponse);
        // **************************

        // Buyer sends Message G to seller.

        // *** SELLER CLOSES TRADE ***
        var sellersCloseTradeResponse = stub.closeTrade(Helloworld.CloseTradeRequest.newBuilder()
                .setTradeId("seller-trade")
                .setMyOutputPeersPrvKeyShare(buyersCloseTradeResponse.getPeerOutputPrvKeyShare())
                .build());
        System.out.println("Got reply: " + sellersCloseTradeResponse);
        // ***************************
    }
}
