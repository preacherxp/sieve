package bookstore.pricing;

import bookstore.catalog.Book;
import java.math.BigDecimal;
import java.util.List;

public class PriceCalculator {
    private final List<PriceRule> rules;
    private final TaxRules tax;

    public PriceCalculator(List<PriceRule> rules, TaxRules tax) {
        this.rules = List.copyOf(rules);
        this.tax = tax;
    }

    public static PriceCalculator standard() {
        return new PriceCalculator(List.of(new MemberDiscount(), new BulkDiscount()), TaxRules.books());
    }

    public BigDecimal total(Book book, int quantity, boolean member) {
        BigDecimal total = book.price().multiply(BigDecimal.valueOf(quantity));
        for (PriceRule rule : rules) {
            total = rule.apply(total, quantity, member);
        }
        return tax.withTax(total);
    }
}
