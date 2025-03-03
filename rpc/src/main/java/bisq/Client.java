package bisq;

import helloworld.GreeterGrpc;
import helloworld.Helloworld;
import io.grpc.Grpc;
import io.grpc.InsecureChannelCredentials;

import java.time.Instant;

public class Client {
    public static void main(String[] args) {
        var channel = Grpc.newChannelBuilderForAddress(
                "127.0.0.1",
                50051,
                InsecureChannelCredentials.create()
        ).build();

        var stub = GreeterGrpc.newBlockingStub(channel);
        var reply = stub.sayHello(Helloworld.HelloRequest.newBuilder()
                .setName("Hello from Java")
                .build());
        System.out.println("Got reply: " + reply);

        var iter = stub.subscribeClock(Helloworld.ClockRequest.newBuilder()
                .setTickPeriodMillis(5000)
                .build());
        iter.forEachRemaining(tickEvent -> System.out.println("Got tick: " +
                Instant.ofEpochMilli(tickEvent.getCurrentTimeMillis())));

        System.out.println("Hello, world!");
    }
}
