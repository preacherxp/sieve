package shop.orders;

import org.springframework.http.HttpStatus;
import org.springframework.http.MediaType;
import org.springframework.http.ResponseEntity;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.PathVariable;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RequestBody;
import org.springframework.web.bind.annotation.RestController;
import reactor.core.publisher.Flux;
import reactor.core.publisher.Mono;
import shop.common.events.OrderPlaced;
import shop.common.web.ApiError;

@RestController
public class OrderController {
    private final OrderService service;
    private final OrderRepository repository;
    private final OrderEvents events;

    public OrderController(OrderService service, OrderRepository repository, OrderEvents events) {
        this.service = service;
        this.repository = repository;
        this.events = events;
    }

    @PostMapping("/orders")
    public Mono<ResponseEntity<Object>> place(@RequestBody OrderRequest request) {
        if (!request.valid()) {
            return Mono.just(ResponseEntity.badRequest().body(new ApiError("INVALID_ORDER", "sku, quantity and email are required")));
        }
        return service.place(request).map(order -> ResponseEntity.status(HttpStatus.CREATED).body(order));
    }

    @GetMapping("/orders/{id}")
    public Mono<ResponseEntity<Order>> get(@PathVariable String id) {
        return repository.findById(id).map(ResponseEntity::ok).defaultIfEmpty(ResponseEntity.notFound().build());
    }

    @GetMapping(path = "/orders/events", produces = MediaType.TEXT_EVENT_STREAM_VALUE)
    public Flux<OrderPlaced> events() {
        return events.stream();
    }
}
