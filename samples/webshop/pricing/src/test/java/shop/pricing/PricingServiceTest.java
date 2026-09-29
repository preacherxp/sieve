package shop.pricing;

import java.util.List;
import org.junit.jupiter.api.Test;
import reactor.test.StepVerifier;
import shop.common.Money;

class PricingServiceTest {
    private final PricingService service =
        new PricingService(List.of(new BulkDiscount(), new CategoryPromotion()), new TaxPolicy());

    @Test
    void picksLargestDiscount() {
        StepVerifier.create(service.quote(new PriceRequest("MUG-1", "merch", Money.of(1000), 12)))
            .expectNext(new PriceQuote(Money.of(12000), Money.of(1800), "category", Money.of(2040), Money.of(12240)))
            .verifyComplete();
    }

    @Test
    void noDiscountForSingleKeyboard() {
        StepVerifier.create(service.quote(new PriceRequest("KBD-1", "hardware", Money.of(12900), 1)))
            .expectNext(new PriceQuote(Money.of(12900), Money.zero(), "none", Money.of(2580), Money.of(15480)))
            .verifyComplete();
    }

    @Test
    void bulkBeatsNothingForHardware() {
        StepVerifier.create(service.quote(new PriceRequest("KBD-1", "hardware", Money.of(12900), 10)).map(PriceQuote::rule))
            .expectNext("bulk")
            .verifyComplete();
    }
}
