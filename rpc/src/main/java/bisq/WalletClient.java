package bisq;

import io.grpc.Grpc;
import io.grpc.InsecureChannelCredentials;
import io.grpc.StatusRuntimeException;
import walletrpc.WalletGrpc;
import walletrpc.WalletOuterClass.ConfRequest;
import walletrpc.WalletOuterClass.ListUnspentRequest;

import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;
import java.util.stream.Collectors;

public class WalletClient {
    public static void main(String[] args) throws InterruptedException {
        var channel = Grpc.newChannelBuilderForAddress(
                "127.0.0.1",
                50051,
                InsecureChannelCredentials.create()
        ).build();

        try {
            var walletStub = WalletGrpc.newBlockingStub(channel);

            System.out.println("Requesting wallet UTXOs.");
            var utxos = walletStub.listUnspent(ListUnspentRequest.newBuilder().build()).getUtxosList();
            utxos.forEach(o -> System.out.println("Got UTXO: " + o));

            System.out.println("Opening tx confidence streams for UTXOs and waiting 5 seconds for events...");
            try (var service = Executors.newVirtualThreadPerTaskExecutor()) {
                var tasks = utxos.stream()
                        .map(o -> Executors.callable(() -> {
                            var confRequest = ConfRequest.newBuilder().setTxId(o.getTxId()).buildPartial();
                            try {
                                walletStub.registerConfidenceNtfn(confRequest)
                                        .forEachRemaining(event -> System.out.println("Got event: " + event));
                            } catch (StatusRuntimeException e) {
                                System.out.println("Got exception: " + e);
                            }
                        }))
                        .collect(Collectors.toList());

                service.invokeAll(tasks, 5, TimeUnit.SECONDS);
            }
            System.out.println("Closed tx confidence stream. Channel still open. Waiting 5 more seconds...");
            TimeUnit.SECONDS.sleep(5);
        } finally {
            System.out.println("Closing channel.");
            channel.shutdown();
        }
    }
}
