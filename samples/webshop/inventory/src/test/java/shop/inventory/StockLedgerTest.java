package shop.inventory;

import org.junit.jupiter.api.Test;
import reactor.test.StepVerifier;

class StockLedgerTest {
    private final StockLedger ledger = new StockLedger();

    @Test
    void takesAvailableUnits() {
        StepVerifier.create(ledger.take("KBD-1", 2).then(ledger.available("KBD-1")))
            .expectNext(1)
            .verifyComplete();
    }

    @Test
    void refusesMoreThanOnHand() {
        StepVerifier.create(ledger.take("KBD-1", 4)).expectNext(false).verifyComplete();
        StepVerifier.create(ledger.available("KBD-1")).expectNext(3).verifyComplete();
    }

    @Test
    void unknownSkuHasNoStock() {
        StepVerifier.create(ledger.take("NOPE", 1)).expectNext(false).verifyComplete();
    }
}
