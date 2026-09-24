package shop.inventory;

import java.util.Map;
import org.springframework.http.HttpStatus;
import org.springframework.http.ResponseEntity;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.PathVariable;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RequestBody;
import org.springframework.web.bind.annotation.RestController;
import reactor.core.publisher.Mono;

@RestController
public class InventoryController {
    private final StockLedger ledger;
    private final ReservationService reservations;

    public InventoryController(StockLedger ledger, ReservationService reservations) {
        this.ledger = ledger;
        this.reservations = reservations;
    }

    @GetMapping("/stock/{sku}")
    public Mono<Map<String, Object>> stock(@PathVariable String sku) {
        return ledger.available(sku).map(units -> Map.of("sku", sku, "available", units));
    }

    @PostMapping("/reservations")
    public Mono<ResponseEntity<Reservation>> reserve(@RequestBody ReservationRequest request) {
        return reservations.reserve(request).map(r -> ResponseEntity.status(HttpStatus.CREATED).body(r));
    }

    @PostMapping("/reservations/{id}/confirm")
    public Mono<ResponseEntity<Reservation>> confirm(@PathVariable String id) {
        return reservations.confirm(id).map(ResponseEntity::ok).defaultIfEmpty(ResponseEntity.notFound().build());
    }
}
