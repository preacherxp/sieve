package shop.pricing;

import java.util.Comparator;
import java.util.List;
import org.springframework.stereotype.Service;
import reactor.core.publisher.Flux;
import reactor.core.publisher.Mono;
import shop.common.Money;

@Service
public class PricingService {
    private final List<PriceRule> rules;
    private final TaxPolicy taxPolicy;

    public PricingService(List<PriceRule> rules, TaxPolicy taxPolicy) {
        this.rules = rules;
        this.taxPolicy = taxPolicy;
    }

    public Mono<PriceQuote> quote(PriceRequest request) {
        Money subtotal = request.subtotal();
        return Flux.fromIterable(rules)
            .map(rule -> new Candidate(rule.name(), rule.discount(request)))
            .filter(candidate -> candidate.discount().cents() > 0)
            .sort(Comparator.comparingLong((Candidate c) -> c.discount().cents()).reversed())
            .next()
            .defaultIfEmpty(new Candidate("none", Money.zero()))
            .map(best -> {
                Money net = subtotal.minus(best.discount());
                Money tax = taxPolicy.tax(request.category(), net);
                return new PriceQuote(subtotal, best.discount(), best.rule(), tax, net.plus(tax));
            });
    }

    private record Candidate(String rule, Money discount) {
    }
}
