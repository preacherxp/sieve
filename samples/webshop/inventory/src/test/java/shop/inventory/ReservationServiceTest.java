package shop.inventory;

import java.time.Duration;
import org.junit.jupiter.api.Test;
import reactor.test.StepVerifier;

class ReservationServiceTest {
    private final StockLedger ledger = new StockLedger();
    private final ReservationService service = new ReservationService(ledger, Duration.ofMinutes(15));

    @Test
    void reservesAndRejectsOversell() {
        StepVerifier.create(service.reserve(new ReservationRequest("BOOK-2", 4)))
            .expectNextMatches(r -> r.quantity() == 4 && !r.confirmed())
            .verifyComplete();
        StepVerifier.create(service.reserve(new ReservationRequest("BOOK-2", 2)))
            .expectError(InsufficientStockException.class)
            .verify();
    }

    @Test
    void unconfirmedReservationReturnsStockAfterTtl() {
        var reservation = service.reserve(new ReservationRequest("BOOK-2", 5)).block();
        StepVerifier.withVirtualTime(() -> service.expireLater(reservation.id()))
            .expectSubscription()
            .expectNoEvent(Duration.ofMinutes(14))
            .thenAwait(Duration.ofMinutes(1))
            .verifyComplete();
        StepVerifier.create(ledger.available("BOOK-2")).expectNext(5).verifyComplete();
    }

    @Test
    void confirmedReservationKeepsStock() {
        var reservation = service.reserve(new ReservationRequest("BOOK-2", 5)).block();
        StepVerifier.create(service.confirm(reservation.id()).map(Reservation::confirmed)).expectNext(true).verifyComplete();
        StepVerifier.withVirtualTime(() -> service.expireLater(reservation.id()))
            .thenAwait(Duration.ofMinutes(15))
            .verifyComplete();
        StepVerifier.create(ledger.available("BOOK-2")).expectNext(0).verifyComplete();
    }
}
