package shop.orders.clients;

import static org.assertj.core.api.Assertions.assertThat;

import org.junit.jupiter.api.Test;
import org.springframework.http.HttpStatus;
import reactor.test.StepVerifier;
import shop.common.Money;

class CatalogClientTest {
    static final String MUG = """
        {"sku":"MUG-1","name":"Lambda mug","category":"merch","price":{"cents":1250,"currency":"EUR"}}""";

    @Test
    void readsProduct() {
        var stub = new StubExchange().on("GET /products/MUG-1", HttpStatus.OK, MUG);
        StepVerifier.create(new CatalogClient(stub.builder(), "http://catalog").product("MUG-1"))
            .expectNext(new ProductView("MUG-1", "Lambda mug", "merch", Money.of(1250)))
            .verifyComplete();
    }

    @Test
    void retriesServerErrors() {
        var stub = new StubExchange()
            .on("GET /products/MUG-1", HttpStatus.SERVICE_UNAVAILABLE, "{}")
            .on("GET /products/MUG-1", HttpStatus.SERVICE_UNAVAILABLE, "{}")
            .on("GET /products/MUG-1", HttpStatus.OK, MUG);
        StepVerifier.create(new CatalogClient(stub.builder(), "http://catalog").product("MUG-1").map(ProductView::sku))
            .expectNext("MUG-1")
            .verifyComplete();
        assertThat(stub.calls("GET /products/MUG-1")).isEqualTo(3);
    }

    @Test
    void doesNotRetryMissingProduct() {
        var stub = new StubExchange().on("GET /products/NOPE", HttpStatus.NOT_FOUND, "{}");
        StepVerifier.create(new CatalogClient(stub.builder(), "http://catalog").product("NOPE"))
            .expectErrorMatches(e -> e instanceof DownstreamException d && d.status().value() == 404)
            .verify();
        assertThat(stub.calls("GET /products/NOPE")).isEqualTo(1);
    }
}
