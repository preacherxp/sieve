package shop.pricing;

import java.util.Map;
import org.springframework.stereotype.Component;
import shop.common.Money;

@Component
public class CategoryPromotion implements PriceRule {
    private final Map<String, Integer> percentByCategory = Map.of("books", 10, "merch", 15);

    @Override
    public String name() {
        return "category";
    }

    @Override
    public Money discount(PriceRequest request) {
        return request.subtotal().percent(percentByCategory.getOrDefault(request.category(), 0));
    }
}
