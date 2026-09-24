package shop.orders.clients;

import static org.assertj.core.api.Assertions.assertThat;

import org.junit.jupiter.api.Test;
import org.springframework.http.HttpStatus;
import reactor.test.StepVerifier;

class InventoryClientTest {
    @Test
    void reservesOnce() {
        var stub = new StubExchange().on("POST /reservations", HttpStatus.CREATED, """
            {"id":"r-1","sku":"MUG-1","quantity":2,"confirmed":false}""");
        StepVerifier.create(new InventoryClient(stub.builder(), "http://inventory").reserve("MUG-1", 2))
            .expectNext(new ReservationView("r-1", "MUG-1", 2, false))
            .verifyComplete();
    }

    @Test
    void neverRetriesReservations() {
        var stub = new StubExchange().on("POST /reservations", HttpStatus.SERVICE_UNAVAILABLE, "{}");
        StepVerifier.create(new InventoryClient(stub.builder(), "http://inventory").reserve("MUG-1", 2))
            .expectError(DownstreamException.class)
            .verify();
        assertThat(stub.calls("POST /reservations")).isEqualTo(1);
    }
}
