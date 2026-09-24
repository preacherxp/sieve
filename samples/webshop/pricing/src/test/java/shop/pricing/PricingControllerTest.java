package shop.pricing;

import org.junit.jupiter.api.Test;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.boot.test.autoconfigure.web.reactive.WebFluxTest;
import org.springframework.context.annotation.Import;
import org.springframework.test.web.reactive.server.WebTestClient;
import shop.common.Money;

@WebFluxTest(PricingController.class)
@Import({PricingService.class, BulkDiscount.class, CategoryPromotion.class, TaxPolicy.class})
class PricingControllerTest {
    @Autowired
    WebTestClient client;

    @Test
    void quotesBooks() {
        client.post().uri("/quotes")
            .bodyValue(new PriceRequest("BOOK-1", "books", Money.of(3999), 2))
            .exchange()
            .expectStatus().isOk()
            .expectBody()
            .jsonPath("$.rule").isEqualTo("category")
            .jsonPath("$.total.cents").isEqualTo(7558);
    }

    @Test
    void rejectsZeroQuantity() {
        client.post().uri("/quotes")
            .bodyValue(new PriceRequest("BOOK-1", "books", Money.of(3999), 0))
            .exchange()
            .expectStatus().isBadRequest();
    }
}
