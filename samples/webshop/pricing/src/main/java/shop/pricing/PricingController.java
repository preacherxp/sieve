package shop.pricing;

import org.springframework.http.HttpStatus;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RequestBody;
import org.springframework.web.bind.annotation.RestController;
import org.springframework.web.server.ResponseStatusException;
import reactor.core.publisher.Mono;

@RestController
public class PricingController {
    private final PricingService service;

    public PricingController(PricingService service) {
        this.service = service;
    }

    @PostMapping("/quotes")
    public Mono<PriceQuote> quote(@RequestBody PriceRequest request) {
        if (request.quantity() <= 0 || request.unitPrice() == null) {
            return Mono.error(new ResponseStatusException(HttpStatus.BAD_REQUEST, "quantity and unitPrice are required"));
        }
        return service.quote(request);
    }
}
