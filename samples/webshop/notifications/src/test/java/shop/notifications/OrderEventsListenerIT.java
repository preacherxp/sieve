package shop.notifications;

import static org.assertj.core.api.Assertions.assertThat;

import java.time.Duration;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.Test;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.boot.test.context.SpringBootTest;
import org.springframework.test.context.DynamicPropertyRegistry;
import org.springframework.test.context.DynamicPropertySource;
import org.springframework.test.web.reactive.server.WebTestClient;
import reactor.core.publisher.Flux;
import reactor.core.publisher.Mono;
import reactor.netty.DisposableServer;
import reactor.netty.http.server.HttpServer;

/** The stub orders stream drops after one event; the listener must reconnect and not send duplicates. */
@SpringBootTest(webEnvironment = SpringBootTest.WebEnvironment.RANDOM_PORT)
class OrderEventsListenerIT {
    static final AtomicInteger CONNECTIONS = new AtomicInteger();
    static final String EVENT = """
        data:{"orderId":"0123456789abcdef","sku":"MUG-1","productName":"Lambda mug","quantity":3,\
        "total":{"cents":3750,"currency":"EUR"},"email":"ada@example.com"}

        """;
    static final DisposableServer ORDERS = HttpServer.create().port(0).route(routes -> routes
        .get("/orders/events", (request, response) -> {
            CONNECTIONS.incrementAndGet();
            return response.header("Content-Type", "text/event-stream")
                .sendString(Flux.just(EVENT).concatWith(Mono.delay(Duration.ofMillis(50)).then(Mono.empty())));
        }))
        .bindNow();

    @DynamicPropertySource
    static void orders(DynamicPropertyRegistry registry) {
        registry.add("services.orders", () -> "http://localhost:" + ORDERS.port());
    }

    @AfterAll
    static void stop() {
        ORDERS.disposeNow();
    }

    @Autowired
    WebTestClient client;

    @Test
    void reconnectsAndSendsEachConfirmationOnce() throws InterruptedException {
        long deadline = System.nanoTime() + Duration.ofSeconds(10).toNanos();
        while (CONNECTIONS.get() < 3 && System.nanoTime() < deadline) {
            Thread.sleep(50);
        }
        assertThat(CONNECTIONS.get()).isGreaterThanOrEqualTo(3);
        client.get().uri("/sent").exchange()
            .expectStatus().isOk()
            .expectBody()
            .jsonPath("$.length()").isEqualTo(1)
            .jsonPath("$[0].subject").isEqualTo("Order 01234567 confirmed");
    }
}
