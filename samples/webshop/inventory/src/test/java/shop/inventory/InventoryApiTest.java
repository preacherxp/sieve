package shop.inventory;

import org.junit.jupiter.api.Test;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.boot.test.context.SpringBootTest;
import org.springframework.test.annotation.DirtiesContext;
import org.springframework.test.web.reactive.server.WebTestClient;

@SpringBootTest(webEnvironment = SpringBootTest.WebEnvironment.RANDOM_PORT)
@DirtiesContext
class InventoryApiTest {
    @Autowired
    WebTestClient client;

    @Test
    void reservesConfirmsAndReportsStock() {
        var reservation = client.post().uri("/reservations").bodyValue(new ReservationRequest("MUG-1", 30)).exchange()
            .expectStatus().isCreated()
            .expectBody(Reservation.class).returnResult().getResponseBody();
        client.post().uri("/reservations/{id}/confirm", reservation.id()).exchange()
            .expectStatus().isOk()
            .expectBody().jsonPath("$.confirmed").isEqualTo(true);
        client.get().uri("/stock/MUG-1").exchange()
            .expectBody().jsonPath("$.available").isEqualTo(70);
    }

    @Test
    void oversellIsConflict() {
        client.post().uri("/reservations").bodyValue(new ReservationRequest("KBD-1", 50)).exchange()
            .expectStatus().isEqualTo(409);
    }
}
