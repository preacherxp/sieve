package shop.inventory;

import java.time.Duration;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.stereotype.Service;
import reactor.core.publisher.Mono;

/** Holds stock for an order. Unconfirmed reservations return their stock after the TTL. */
@Service
public class ReservationService {
    private final StockLedger ledger;
    private final Duration ttl;
    private final Map<String, Reservation> reservations = new ConcurrentHashMap<>();

    public ReservationService(StockLedger ledger, @Value("${inventory.reservation-ttl:15m}") Duration ttl) {
        this.ledger = ledger;
        this.ttl = ttl;
    }

    public Mono<Reservation> reserve(ReservationRequest request) {
        if (request.quantity() <= 0) {
            return Mono.error(new IllegalArgumentException("quantity must be positive"));
        }
        return ledger.take(request.sku(), request.quantity())
            .flatMap(taken -> taken
                ? Mono.just(new Reservation(UUID.randomUUID().toString(), request.sku(), request.quantity(), false))
                : Mono.error(new InsufficientStockException(request.sku(), request.quantity())))
            .doOnNext(reservation -> {
                reservations.put(reservation.id(), reservation);
                expireLater(reservation.id()).subscribe();
            });
    }

    public Mono<Reservation> confirm(String id) {
        return Mono.justOrEmpty(reservations.computeIfPresent(id, (key, reservation) -> reservation.confirm()));
    }

    /** Completes once the reservation's TTL has passed, releasing its stock unless it was confirmed. */
    Mono<Void> expireLater(String id) {
        return Mono.delay(ttl).then(Mono.defer(() -> {
            Reservation reservation = reservations.get(id);
            if (reservation == null || reservation.confirmed()) {
                return Mono.empty();
            }
            reservations.remove(id);
            return ledger.put(reservation.sku(), reservation.quantity());
        }));
    }
}
