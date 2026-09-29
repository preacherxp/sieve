package shop.orders.clients;

import org.junit.jupiter.api.Test;
import org.springframework.http.HttpStatus;
import reactor.test.StepVerifier;
import shop.common.Money;

class PricingClientTest {
    @Test
    void readsQuote() {
        var stub = new StubExchange().on("POST /quotes", HttpStatus.OK, """
            {"subtotal":{"cents":2500,"currency":"EUR"},"discount":{"cents":375,"currency":"EUR"},"rule":"category",
             "tax":{"cents":425,"currency":"EUR"},"total":{"cents":2550,"currency":"EUR"}}""");
        var mug = new ProductView("MUG-1", "Lambda mug", "merch", Money.of(1250));
        StepVerifier.create(new PricingClient(stub.builder(), "http://pricing").quote(mug, 2))
            .expectNextMatches(quote -> quote.total().equals(Money.of(2550)) && quote.rule().equals("category"))
            .verifyComplete();
    }

    @Test
    void giveUpAfterTwoRetries() {
        var stub = new StubExchange().on("POST /quotes", HttpStatus.BAD_GATEWAY, "{}");
        var mug = new ProductView("MUG-1", "Lambda mug", "merch", Money.of(1250));
        StepVerifier.create(new PricingClient(stub.builder(), "http://pricing").quote(mug, 2))
            .expectError(DownstreamException.class)
            .verify();
        org.assertj.core.api.Assertions.assertThat(stub.calls("POST /quotes")).isEqualTo(3);
    }
}
