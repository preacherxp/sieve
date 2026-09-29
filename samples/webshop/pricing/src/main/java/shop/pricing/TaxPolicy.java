package shop.pricing;

import org.springframework.stereotype.Component;
import shop.common.Money;

@Component
public class TaxPolicy {
    public int ratePercent(String category) {
        return switch (category) {
            case "books" -> 5;
            default -> 20;
        };
    }

    public Money tax(String category, Money net) {
        return net.percent(ratePercent(category));
    }
}
