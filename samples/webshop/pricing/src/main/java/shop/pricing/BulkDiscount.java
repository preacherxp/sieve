package shop.pricing;

import org.springframework.stereotype.Component;
import shop.common.Money;

@Component
public class BulkDiscount implements PriceRule {
    static final int THRESHOLD = 10;

    @Override
    public String name() {
        return "bulk";
    }

    @Override
    public Money discount(PriceRequest request) {
        return request.quantity() >= THRESHOLD ? request.subtotal().percent(5) : Money.zero();
    }
}
